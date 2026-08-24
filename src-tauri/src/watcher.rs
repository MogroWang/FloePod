//! 暂存文件夹监听：用户在资源管理器手动增删文件时对账数据库（每个匣独立监听）。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, Manager};

use crate::db::StagedItem;
use crate::events;
use crate::manager;
use crate::settings::{self, Settings};
use crate::state::AppState;

const WATCH_INSTALL_RETRY_INTERVAL: Duration = Duration::from_secs(10);
static WATCH_INSTALL_RETRY_NEEDED: AtomicBool = AtomicBool::new(false);

fn current_settings(app: &AppHandle) -> Result<Settings, String> {
    let state = app.state::<AppState>();
    let settings = {
        let conn = state.db.lock().unwrap();
        settings::load(
            &conn,
            &state.data_dir.to_string_lossy(),
            env!("CARGO_PKG_VERSION"),
        )?
    };
    settings::validate(&settings, &state.data_dir.to_string_lossy())?;
    Ok(settings)
}

/// 常驻对账线程：有脏标记且非应用自身写入后，整盘对账。
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let mut retrying_unavailable_folder = false;
        let mut last_watch_install_retry = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(800));
            // 安装 watcher 失败时通常不会再有文件事件来唤醒本线程。用独立、低频的
            // 重试时钟恢复可移动盘、临时权限/句柄耗尽等故障，避免永久失去监听。
            if WATCH_INSTALL_RETRY_NEEDED.load(Ordering::Relaxed)
                && last_watch_install_retry.elapsed() >= WATCH_INSTALL_RETRY_INTERVAL
            {
                last_watch_install_retry = Instant::now();
                restart_all(&app);
            }
            let state = app.state::<AppState>();
            if !state.watcher_dirty.swap(false, Ordering::Relaxed) {
                continue;
            }
            if state.staged_recently() {
                // 应用自身写入的 notify 事件可以延后，但不能清掉同一窗口内真实的外部变化。
                state.watcher_dirty.store(true, Ordering::Relaxed);
                continue;
            }
            match reconcile_all(&app) {
                Ok(()) => {
                    if retrying_unavailable_folder {
                        // 可移动盘恢复后，重新安装此前创建失败的目录监听器。
                        restart_all(&app);
                        retrying_unavailable_folder = false;
                    }
                }
                Err(e) => {
                    eprintln!("[watcher] 对账部分失败: {e}");
                    retrying_unavailable_folder = true;
                    // 离线盘不应造成 800ms 的日志/磁盘忙循环；仍需周期重试以发现恢复。
                    std::thread::sleep(Duration::from_secs(4));
                    app.state::<AppState>()
                        .watcher_dirty
                        .store(true, Ordering::Relaxed);
                }
            }
        }
    });
}

/// 按当前设置重建所有匣的监听。
pub fn restart_all(app: &AppHandle) {
    // Serialize snapshot + replacement as one operation. If a retry reads old settings before a
    // concurrent update installs new watchers, it must not later clear them and reinstall S1.
    let state = app.state::<AppState>();
    let mut guard = state.watcher.lock().unwrap();
    let settings = match current_settings(app) {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("[watcher] 配置无效，已停止目录监听: {e}");
            guard.clear();
            WATCH_INSTALL_RETRY_NEEDED.store(false, Ordering::Relaxed);
            return;
        }
    };
    let folders: Vec<(u64, String)> = settings
        .pods
        .iter()
        .filter(|p| p.enabled && !p.staging_folder.is_empty())
        .map(|p| (p.id, p.staging_folder.clone()))
        .collect();
    WATCH_INSTALL_RETRY_NEEDED.store(false, Ordering::Relaxed);
    guard.clear();

    for (pod_id, path) in folders {
        let dir = PathBuf::from(&path);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("[watcher] 无法创建暂存目录 {}: {e}", dir.display());
            WATCH_INSTALL_RETRY_NEEDED.store(true, Ordering::Relaxed);
            continue;
        }
        let app2 = app.clone();
        match notify::recommended_watcher(move |result| {
            if let Err(e) = result {
                eprintln!("[watcher] 目录监听器运行失败: {e}");
                WATCH_INSTALL_RETRY_NEEDED.store(true, Ordering::Relaxed);
            }
            let st = app2.state::<AppState>();
            st.watcher_dirty.store(true, Ordering::Relaxed);
        }) {
            Ok(mut watcher) => match watcher.watch(&dir, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    guard.insert(pod_id, watcher);
                }
                Err(e) => {
                    eprintln!("[watcher] 无法监听暂存目录 {}: {e}", dir.display());
                    WATCH_INSTALL_RETRY_NEEDED.store(true, Ordering::Relaxed);
                }
            },
            Err(e) => {
                eprintln!("[watcher] 无法创建目录监听器 {}: {e}", dir.display());
                WATCH_INSTALL_RETRY_NEEDED.store(true, Ordering::Relaxed);
            }
        }
    }
}

fn reconcile_all(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    // 与选名、复制/移动和 SQLite 入库共用同一锁，绝不扫描半成品。
    // 全仓统一锁顺序为 file_ops -> db。
    let _file_operation = state.file_ops.lock().unwrap();
    let settings = current_settings(app)?;
    let folders: Vec<(u64, String)> = settings
        .pods
        .iter()
        .filter(|p| p.enabled)
        .map(|p| (p.id, p.staging_folder.clone()))
        .collect();

    let mut changed_pods = HashSet::new();
    let mut folder_errors = Vec::new();

    for (pod_id, folder) in folders {
        if folder.is_empty() {
            continue;
        }
        let snapshot = (|| -> Result<(PathBuf, Vec<StagedItem>, HashSet<String>), String> {
            let folder = settings::resolve_path(&PathBuf::from(&folder))?;
            if !folder.is_dir() {
                return Err(format!("暂存目录不存在或不可用: {}", folder.display()));
            }

            // 只有完整、无错误的目录快照才允许做“磁盘没有 -> 删除数据库记录”。
            let entries = std::fs::read_dir(&folder)
                .map_err(|e| format!("无法读取暂存目录 {}: {e}", folder.display()))?;
            let mut observed = Vec::new();
            let mut unsafe_keys = HashSet::new();
            for result in entries {
                let entry = result.map_err(|e| format!("读取目录项失败: {e}"))?;
                let raw_path = entry.path();
                let entry_name = entry.file_name();
                let internal_name = entry_name.to_string_lossy();
                if internal_name.starts_with(".floepod-inflight-")
                    || internal_name.starts_with(".floepod-move-source-")
                {
                    // Reserved transactional paths are never user-visible items.
                    // They may survive a denied cleanup and must not be indexed as
                    // a second staged copy after the operation returns.
                    unsafe_keys.insert(settings::path_key(&raw_path));
                    continue;
                }
                let meta = std::fs::symlink_metadata(&raw_path)
                    .map_err(|e| format!("读取 {} 元数据失败: {e}", raw_path.display()))?;
                let direct_path = folder.join(&entry_name);
                if crate::commands::is_reparse_or_symlink(&meta) {
                    // 不跟随 symlink / junction，也不把它作为可删除的暂存条目暴露给 UI。
                    // 物理目录项原样保留；若旧版本曾索引它，下面只清理数据库记录。
                    unsafe_keys.insert(settings::path_key(&direct_path));
                    continue;
                }
                let path = settings::resolve_path(&direct_path)?;
                if !settings::path_is_within(&path, &folder)
                    || settings::paths_equal(&path, &folder)
                {
                    return Err(format!("目录项越出暂存目录: {}", path.display()));
                }
                let name = entry_name.to_string_lossy().to_string();
                let ext = crate::commands::ext_of(&name);
                let kind = if meta.is_dir() {
                    "folder"
                } else if ext.as_deref() == Some("lnk") {
                    "shortcut"
                } else {
                    "file"
                };
                observed.push(StagedItem {
                    id: 0,
                    pod_id: pod_id as i64,
                    kind: kind.into(),
                    staging_path: path.to_string_lossy().to_string(),
                    original_path: None,
                    name,
                    ext,
                    size: if meta.is_dir() { 0 } else { meta.len() as i64 },
                    created_at: crate::db::now_ms(),
                });
            }
            Ok((folder, observed, unsafe_keys))
        })();
        let (folder, observed, unsafe_keys) = match snapshot {
            Ok(snapshot) => snapshot,
            Err(e) => {
                folder_errors.push(format!("匣 {pod_id}: {e}"));
                continue;
            }
        };

        let known_items = {
            let conn = state.db.lock().unwrap();
            crate::db::items_of_pod(&conn, pod_id as i64)?
        };
        let mut known: HashMap<String, StagedItem> = HashMap::new();
        let mut invalid_ids = Vec::new();
        for item in known_items {
            let raw = PathBuf::from(&item.staging_path);
            let Some(name) = raw.file_name() else {
                invalid_ids.push(item.id);
                continue;
            };
            let Some(parent) = raw.parent() else {
                invalid_ids.push(item.id);
                continue;
            };
            let Ok(parent) = settings::resolve_path(parent) else {
                // 暂时无法访问时保留索引，不能把权限错误等同于“文件不存在”。
                continue;
            };
            let safe_path = parent.join(name);
            if !settings::paths_equal(&parent, &folder) {
                invalid_ids.push(item.id);
                continue;
            }
            let key = settings::path_key(&safe_path);
            let unsafe_entry = unsafe_keys.contains(&key)
                || std::fs::symlink_metadata(&raw)
                    .map(|meta| crate::commands::is_reparse_or_symlink(&meta))
                    .unwrap_or(false);
            if unsafe_entry {
                invalid_ids.push(item.id);
                continue;
            }
            if let Some(duplicate) = known.insert(key, item) {
                invalid_ids.push(duplicate.id);
            }
        }

        let mut changed = false;
        let mut conn = state.db.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for disk in observed {
            let key = settings::path_key(PathBuf::from(&disk.staging_path).as_path());
            if let Some(existing) = known.remove(&key) {
                let observed_kind = if existing.kind == "text"
                    && disk.kind == "file"
                    && disk.ext.as_deref() == Some("txt")
                {
                    "text"
                } else {
                    disk.kind.as_str()
                };
                if existing.kind != observed_kind
                    || existing.staging_path != disk.staging_path
                    || existing.name != disk.name
                    || existing.ext != disk.ext
                    || existing.size != disk.size
                {
                    crate::db::update_item_observed(
                        &tx,
                        existing.id,
                        observed_kind,
                        &disk.staging_path,
                        &disk.name,
                        disk.ext.as_deref(),
                        disk.size,
                    )?;
                    changed = true;
                }
            } else {
                crate::db::insert_item(&tx, &disk)?;
                changed = true;
            }
        }

        invalid_ids.extend(
            known
                .values()
                .filter(|item| {
                    matches!(
                        std::fs::symlink_metadata(&item.staging_path),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound
                    )
                })
                .map(|item| item.id),
        );
        invalid_ids.sort_unstable();
        invalid_ids.dedup();
        if !invalid_ids.is_empty() {
            crate::db::delete_items_by_ids(&tx, &invalid_ids)?;
            changed = true;
        }
        tx.commit().map_err(|e| e.to_string())?;
        if changed {
            changed_pods.insert(pod_id);
        }
    }

    if !changed_pods.is_empty() {
        for pod in settings
            .pods
            .iter()
            .filter(|p| p.enabled && changed_pods.contains(&p.id))
        {
            let payload = serde_json::json!({ "podId": pod.id });
            if manager::pod_panel(app, pod.id).is_some() {
                let _ = app.emit_to(
                    format!("pod_{}_panel", pod.id),
                    events::ITEMS_CHANGED,
                    payload.clone(),
                );
            }
            if manager::pod_bar(app, pod.id).is_some() {
                let _ = app.emit_to(format!("pod_{}", pod.id), events::ITEMS_CHANGED, payload);
            }
        }
    }
    if folder_errors.is_empty() {
        Ok(())
    } else {
        Err(folder_errors.join("；"))
    }
}
