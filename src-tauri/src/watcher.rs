//! Per-pod staging directory watchers and SQLite reconciliation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use rusqlite::Connection;
use tauri::{AppHandle, Manager};

use crate::db::{self, StagedItem};
use crate::events;
use crate::file_ops;
use crate::settings::{self, Settings};
use crate::state::AppState;

const INSTALL_RETRY_INTERVAL: Duration = Duration::from_secs(10);
static INSTALL_RETRY_NEEDED: AtomicBool = AtomicBool::new(false);

fn current_settings(app: &AppHandle) -> Result<Settings, String> {
    let state = app.state::<AppState>();
    let current = {
        let connection = state.db.lock().unwrap();
        settings::load(
            &connection,
            &state.data_dir.to_string_lossy(),
            env!("CARGO_PKG_VERSION"),
        )?
    };
    settings::validate(&current, &state.data_dir.to_string_lossy())?;
    Ok(current)
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let mut recovering_unavailable_folder = false;
        let mut last_install_retry = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(800));
            if INSTALL_RETRY_NEEDED.load(Ordering::Relaxed)
                && last_install_retry.elapsed() >= INSTALL_RETRY_INTERVAL
            {
                last_install_retry = Instant::now();
                restart_all(&app);
            }
            let state = app.state::<AppState>();
            if !state.watcher_dirty.swap(false, Ordering::Relaxed) {
                continue;
            }
            if state.staged_recently() {
                // Delay our own notify events without discarding an external
                // change that arrived in the same suppression window.
                state.watcher_dirty.store(true, Ordering::Relaxed);
                continue;
            }
            match reconcile_all(&app) {
                Ok(()) => {
                    if recovering_unavailable_folder {
                        restart_all(&app);
                        recovering_unavailable_folder = false;
                    }
                }
                Err(error) => {
                    crate::logging::write(&format!("[watcher] 对账部分失败: {error}"));
                    recovering_unavailable_folder = true;
                    std::thread::sleep(Duration::from_secs(4));
                    state.watcher_dirty.store(true, Ordering::Relaxed);
                }
            }
        }
    });
}

pub fn restart_all(app: &AppHandle) {
    // Serialize the settings snapshot with watcher replacement. A retry that
    // read old settings must never clear a newer set installed concurrently.
    let state = app.state::<AppState>();
    let mut watchers = state.watcher.lock().unwrap();
    let current = match current_settings(app) {
        Ok(current) => current,
        Err(error) => {
            crate::logging::write(&format!("[watcher] 配置无效，已停止目录监听: {error}"));
            watchers.clear();
            INSTALL_RETRY_NEEDED.store(false, Ordering::Relaxed);
            return;
        }
    };
    let folders: Vec<_> = current
        .pods
        .iter()
        .filter(|pod| pod.enabled && !pod.staging_folder.is_empty())
        .map(|pod| (pod.id, pod.staging_folder.clone()))
        .collect();
    INSTALL_RETRY_NEEDED.store(false, Ordering::Relaxed);
    watchers.clear();

    for (pod_id, folder) in folders {
        let directory = PathBuf::from(folder);
        if let Err(error) = std::fs::create_dir_all(&directory) {
            crate::logging::write(&format!(
                "[watcher] 无法创建暂存目录 {}: {error}",
                directory.display()
            ));
            INSTALL_RETRY_NEEDED.store(true, Ordering::Relaxed);
            continue;
        }
        let callback_app = app.clone();
        match notify::recommended_watcher(move |result| {
            if let Err(error) = result {
                crate::logging::write(&format!("[watcher] 目录监听器运行失败: {error}"));
                INSTALL_RETRY_NEEDED.store(true, Ordering::Relaxed);
            }
            callback_app
                .state::<AppState>()
                .watcher_dirty
                .store(true, Ordering::Relaxed);
        }) {
            Ok(mut watcher) => match watcher.watch(&directory, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    watchers.insert(pod_id, watcher);
                }
                Err(error) => {
                    crate::logging::write(&format!(
                        "[watcher] 无法监听暂存目录 {}: {error}",
                        directory.display()
                    ));
                    INSTALL_RETRY_NEEDED.store(true, Ordering::Relaxed);
                }
            },
            Err(error) => {
                crate::logging::write(&format!(
                    "[watcher] 无法创建目录监听器 {}: {error}",
                    directory.display()
                ));
                INSTALL_RETRY_NEEDED.store(true, Ordering::Relaxed);
            }
        }
    }
}

struct DirectorySnapshot {
    root: PathBuf,
    observed: Vec<StagedItem>,
    unsafe_keys: HashSet<String>,
}

fn read_snapshot(pod_id: u64, configured_folder: &Path) -> Result<DirectorySnapshot, String> {
    let root = settings::resolve_path(configured_folder)?;
    if !root.is_dir() {
        return Err(format!("暂存目录不存在或不可用: {}", root.display()));
    }
    let entries = std::fs::read_dir(&root)
        .map_err(|error| format!("无法读取暂存目录 {}: {error}", root.display()))?;
    let mut observed = Vec::new();
    let mut unsafe_keys = HashSet::new();
    for result in entries {
        let entry = result.map_err(|error| format!("读取目录项失败: {error}"))?;
        let raw_path = entry.path();
        let name = entry.file_name();
        let internal_name = name.to_string_lossy();
        if internal_name.starts_with(".floepod-inflight-")
            || internal_name.starts_with(".floepod-move-source-")
        {
            unsafe_keys.insert(settings::path_key(&raw_path));
            continue;
        }
        let metadata = std::fs::symlink_metadata(&raw_path)
            .map_err(|error| format!("读取 {} 元数据失败: {error}", raw_path.display()))?;
        let direct_path = root.join(&name);
        if file_ops::is_reparse_or_symlink(&metadata) {
            unsafe_keys.insert(settings::path_key(&direct_path));
            continue;
        }
        let path = settings::resolve_path(&direct_path)?;
        if !settings::path_is_within(&path, &root) || settings::paths_equal(&path, &root) {
            return Err(format!("目录项越出暂存目录: {}", path.display()));
        }
        let name = name.to_string_lossy().to_string();
        let extension = file_ops::extension(&name);
        let kind = if metadata.is_dir() {
            "folder"
        } else if extension.as_deref() == Some("lnk") {
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
            ext: extension,
            size: if metadata.is_dir() {
                0
            } else {
                metadata.len() as i64
            },
            created_at: db::now_ms(),
        });
    }
    Ok(DirectorySnapshot {
        root,
        observed,
        unsafe_keys,
    })
}

fn reconcile_pod(
    connection: &mut Connection,
    pod_id: u64,
    snapshot: DirectorySnapshot,
) -> Result<bool, String> {
    let known_items = db::items_of_pod(connection, pod_id as i64)?;
    let mut known = HashMap::new();
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
            // Access errors do not prove deletion. Preserve this row until a
            // complete readable snapshot can compare it safely.
            continue;
        };
        let safe_path = parent.join(name);
        if !settings::paths_equal(&parent, &snapshot.root) {
            invalid_ids.push(item.id);
            continue;
        }
        let key = settings::path_key(&safe_path);
        let unsafe_entry = snapshot.unsafe_keys.contains(&key)
            || std::fs::symlink_metadata(&raw)
                .map(|metadata| file_ops::is_reparse_or_symlink(&metadata))
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
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    for disk in snapshot.observed {
        let key = settings::path_key(Path::new(&disk.staging_path));
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
                db::update_item_observed(
                    &transaction,
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
            db::insert_item(&transaction, &disk)?;
            changed = true;
        }
    }
    invalid_ids.extend(
        known
            .values()
            .filter(|item| {
                matches!(
                    std::fs::symlink_metadata(&item.staging_path),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound
                )
            })
            .map(|item| item.id),
    );
    invalid_ids.sort_unstable();
    invalid_ids.dedup();
    if !invalid_ids.is_empty() {
        db::delete_items_by_ids(&transaction, &invalid_ids)?;
        changed = true;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(changed)
}

fn reconcile_all(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _operation = state.file_ops.lock().unwrap();
    let current = current_settings(app)?;
    let folders: Vec<_> = current
        .pods
        .iter()
        .filter(|pod| pod.enabled)
        .map(|pod| (pod.id, pod.staging_folder.clone()))
        .collect();
    let mut changed_pods = HashSet::new();
    let mut errors = Vec::new();
    for (pod_id, folder) in folders {
        if folder.is_empty() {
            continue;
        }
        let snapshot = match read_snapshot(pod_id, Path::new(&folder)) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                errors.push(format!("匣 {pod_id}: {error}"));
                continue;
            }
        };
        let changed = {
            let mut connection = state.db.lock().unwrap();
            reconcile_pod(&mut connection, pod_id, snapshot)?
        };
        if changed {
            changed_pods.insert(pod_id);
        }
    }
    for pod_id in changed_pods {
        events::emit_items_changed(app, pod_id);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        db::migrate(&connection).unwrap();
        connection
    }

    #[test]
    fn external_add_update_and_delete_reconcile_without_losing_text_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("note.txt");
        std::fs::write(&path, b"one").unwrap();
        let mut connection = connection();

        assert!(reconcile_pod(
            &mut connection,
            1,
            read_snapshot(1, temporary.path()).unwrap()
        )
        .unwrap());
        let item = db::items_of_pod(&connection, 1).unwrap().remove(0);
        assert_eq!(item.kind, "file");
        db::update_item_observed(
            &connection,
            item.id,
            "text",
            &item.staging_path,
            &item.name,
            item.ext.as_deref(),
            item.size,
        )
        .unwrap();

        std::fs::write(&path, b"a longer text").unwrap();
        assert!(reconcile_pod(
            &mut connection,
            1,
            read_snapshot(1, temporary.path()).unwrap()
        )
        .unwrap());
        let updated = db::items_of_pod(&connection, 1).unwrap().remove(0);
        assert_eq!(updated.kind, "text");
        assert_eq!(updated.created_at, item.created_at);
        assert_eq!(updated.size, 13);

        std::fs::remove_file(path).unwrap();
        assert!(reconcile_pod(
            &mut connection,
            1,
            read_snapshot(1, temporary.path()).unwrap()
        )
        .unwrap());
        assert!(db::items_of_pod(&connection, 1).unwrap().is_empty());
    }

    #[test]
    fn reconciliation_isolated_by_pod_and_ignores_internal_paths() {
        let temporary = tempfile::tempdir().unwrap();
        let first = temporary.path().join("first");
        let second = temporary.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("a.txt"), b"a").unwrap();
        std::fs::write(first.join(".floepod-inflight-1-1"), b"partial").unwrap();
        std::fs::write(second.join("b.txt"), b"b").unwrap();
        let mut connection = connection();

        reconcile_pod(&mut connection, 1, read_snapshot(1, &first).unwrap()).unwrap();
        assert_eq!(db::items_of_pod(&connection, 1).unwrap().len(), 1);
        assert!(db::items_of_pod(&connection, 2).unwrap().is_empty());
        reconcile_pod(&mut connection, 2, read_snapshot(2, &second).unwrap()).unwrap();
        assert_eq!(db::items_of_pod(&connection, 1).unwrap().len(), 1);
        assert_eq!(db::items_of_pod(&connection, 2).unwrap().len(), 1);

        std::fs::remove_file(first.join("a.txt")).unwrap();
        reconcile_pod(&mut connection, 1, read_snapshot(1, &first).unwrap()).unwrap();
        assert!(db::items_of_pod(&connection, 1).unwrap().is_empty());
        assert_eq!(db::items_of_pod(&connection, 2).unwrap().len(), 1);
    }

    #[test]
    fn unreadable_or_missing_root_never_becomes_an_empty_snapshot() {
        let temporary = tempfile::tempdir().unwrap();
        let missing = temporary.path().join("offline");
        assert!(read_snapshot(1, &missing).is_err());
    }
}
