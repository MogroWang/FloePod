//! Tauri 命令层：匣 / 暂存 / 导出 / 设置 / 窗口编排。

use std::collections::{hash_map::RandomState, HashMap, HashSet};
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::db::{self, StagedItem};
use crate::events;
use crate::lnk;
use crate::manager;
use crate::settings::{self, Pod, Settings};
use crate::state::{AppState, DragCutEntry, DragCutFileIdentity, DragCutSnapshot};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEBUG_LOG_MAX_BYTES: u64 = 2 * 1024 * 1024;
const DEBUG_LOG_MAX_MESSAGE_CHARS: usize = 16 * 1024;
static DEBUG_LOG_LOCK: Mutex<()> = Mutex::new(());
static FILE_OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const DRAG_CUT_TOKEN_TTL: Duration = Duration::from_secs(5 * 60);

fn data_dir_str(state: &AppState) -> String {
    state.data_dir.to_string_lossy().to_string()
}

fn load_settings(state: &AppState) -> Result<Settings, String> {
    let conn = state.db.lock().unwrap();
    settings::load(&conn, &data_dir_str(state), VERSION)
}

fn load_settings_conn(conn: &rusqlite::Connection, state: &AppState) -> Result<Settings, String> {
    settings::load(conn, &data_dir_str(state), VERSION)
}

/// 将同步文件系统/图片任务移出 Tauri 的异步命令执行线程。
async fn run_blocking_command<T, F>(label: &'static str, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| format!("{label}后台任务异常终止：{error}"))?
}

fn basic_drag_cut_identity(metadata: &fs::Metadata) -> DragCutFileIdentity {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        DragCutFileIdentity {
            // std 1.97 的 by-handle 文件 ID API 仍不稳定；字段保留为 Option，
            // 当前用稳定的 creation/write/size/type 组合执行保守身份校验。
            volume_serial_number: None,
            file_index: None,
            creation_time: metadata.creation_time(),
            last_write_time: metadata.last_write_time(),
            size: metadata.file_size(),
            is_file: metadata.file_type().is_file(),
            is_dir: metadata.file_type().is_dir(),
            tree_fingerprint: None,
        }
    }

    #[cfg(not(windows))]
    {
        fn timestamp(value: io::Result<std::time::SystemTime>) -> u64 {
            value
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
                .unwrap_or(0)
        }

        DragCutFileIdentity {
            volume_serial_number: None,
            file_index: None,
            creation_time: timestamp(metadata.created()),
            last_write_time: timestamp(metadata.modified()),
            size: metadata.len(),
            is_file: metadata.file_type().is_file(),
            is_dir: metadata.file_type().is_dir(),
            tree_fingerprint: None,
        }
    }
}

const DRAG_CUT_MAX_TREE_ENTRIES: usize = 100_000;
const FNV64_OFFSET: u64 = 0xcbf29ce484222325;
const FNV64_PRIME: u64 = 0x100000001b3;

fn fingerprint_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV64_PRIME);
    }
}

fn fingerprint_u64(hash: &mut u64, value: u64) {
    fingerprint_bytes(hash, &value.to_le_bytes());
}

fn fingerprint_name(hash: &mut u64, name: &std::ffi::OsStr) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for value in name.encode_wide() {
            fingerprint_bytes(hash, &value.to_le_bytes());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        fingerprint_bytes(hash, name.as_bytes());
    }
    #[cfg(not(any(windows, unix)))]
    fingerprint_bytes(hash, name.to_string_lossy().as_bytes());
}

fn fingerprint_metadata(hash: &mut u64, metadata: &fs::Metadata) {
    let identity = basic_drag_cut_identity(metadata);
    fingerprint_u64(hash, identity.creation_time);
    fingerprint_u64(hash, identity.last_write_time);
    fingerprint_u64(hash, identity.size);
    fingerprint_bytes(
        hash,
        &[u8::from(identity.is_file), u8::from(identity.is_dir)],
    );
}

fn fingerprint_directory(
    directory: &Path,
    hash: &mut u64,
    count: &mut usize,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("无法读取目录树 {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法读取目录树 {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| settings::path_key(&entry.path()));

    for entry in entries {
        *count += 1;
        if *count > DRAG_CUT_MAX_TREE_ENTRIES {
            return Err(format!(
                "目录树超过 {DRAG_CUT_MAX_TREE_ENTRIES} 项，无法安全剪切拖出"
            ));
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("无法读取目录项 {}: {error}", path.display()))?;
        if is_reparse_or_symlink(&metadata) {
            return Err(format!(
                "目录树包含符号链接或目录重解析点: {}",
                path.display()
            ));
        }
        fingerprint_bytes(hash, &[0x01]);
        fingerprint_name(hash, &entry.file_name());
        fingerprint_bytes(hash, &[0x00]);
        fingerprint_metadata(hash, &metadata);
        if metadata.is_dir() {
            fingerprint_bytes(hash, &[0x02]);
            fingerprint_directory(&path, hash, count)?;
            fingerprint_bytes(hash, &[0x03]);
        }
    }
    Ok(())
}

fn drag_cut_identity(path: &Path, metadata: &fs::Metadata) -> Result<DragCutFileIdentity, String> {
    let mut identity = basic_drag_cut_identity(metadata);
    if metadata.is_dir() {
        let mut hash = FNV64_OFFSET;
        let mut count = 0;
        fingerprint_directory(path, &mut hash, &mut count)?;
        fingerprint_u64(&mut hash, count as u64);
        identity.tree_fingerprint = Some(hash);
    }
    Ok(identity)
}

fn new_drag_cut_token(state: &AppState) -> String {
    let sequence = state
        .next_drag_cut_token
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    // RandomState 使用进程内随机密钥；序号保证同一进程内唯一，随机摘要避免令牌可猜。
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(sequence);
    hasher.write(&timestamp.to_le_bytes());
    format!("cut-{sequence:016x}-{:016x}", hasher.finish())
}

fn store_drag_cut_snapshot(state: &AppState, entries: Vec<DragCutEntry>) -> String {
    let now = Instant::now();
    let token = new_drag_cut_token(state);
    let mut snapshots = state.drag_cut_tokens.lock().unwrap();
    snapshots.retain(|_, snapshot| snapshot.expires_at > now);
    snapshots.insert(
        token.clone(),
        DragCutSnapshot {
            expires_at: now + DRAG_CUT_TOKEN_TTL,
            entries,
        },
    );
    token
}

fn take_drag_cut_snapshot(state: &AppState, token: &str) -> Result<DragCutSnapshot, String> {
    if token.trim().is_empty() {
        return Err("剪切令牌为空".into());
    }
    let now = Instant::now();
    let mut snapshots = state.drag_cut_tokens.lock().unwrap();
    let snapshot = snapshots.remove(token);
    snapshots.retain(|_, snapshot| snapshot.expires_at > now);
    let snapshot = snapshot.ok_or_else(|| "剪切令牌无效或已被使用".to_string())?;
    if snapshot.expires_at <= now {
        return Err("剪切令牌已过期，请重新拖动".into());
    }
    Ok(snapshot)
}

/* ---------- 工具 ---------- */

pub fn ext_of(name: &str) -> Option<String> {
    let idx = name.rfind('.')?;
    if idx == 0 {
        return None; // ".gitignore" 之类视为无扩展名
    }
    Some(name[idx + 1..].to_ascii_lowercase())
}

/// 目标目录内唯一文件名：`a.pdf` -> `a (2).pdf`
pub fn unique_target(
    dir: &Path,
    desired: &str,
    used: &mut HashSet<String>,
) -> Result<PathBuf, String> {
    let mut name = desired.to_string();
    let mut n = 1;
    loop {
        let candidate = dir.join(&name);
        let key = settings::path_key(&settings::resolve_path(&candidate)?);
        // `exists()` 会把 dangling symlink 当成不存在，后续写入可能沿链接越出暂存目录。
        match fs::symlink_metadata(&candidate) {
            Err(e) if e.kind() == io::ErrorKind::NotFound && !used.contains(&key) => {
                used.insert(key);
                return Ok(candidate);
            }
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(format!("无法检查目标路径 {}: {e}", candidate.display()));
            }
            _ => {}
        }
        n += 1;
        let (stem, ext) = match desired.rfind('.') {
            Some(i) if i > 0 => (&desired[..i], &desired[i..]),
            _ => (desired, ""),
        };
        name = format!("{stem} ({n}){ext}");
    }
}

pub(crate) fn is_reparse_or_symlink(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT：包括 junction 等目录重解析点。
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn copy_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if is_reparse_or_symlink(&meta) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("不支持复制符号链接或目录重解析点: {}", src.display()),
        ));
    }
    if meta.is_dir() {
        // `unique_target` 的检查与真正创建之间仍可能有外部竞争；目标必须独占创建，
        // 不能把目录合并进刚出现的同名路径，也不能截断同名文件。
        fs::create_dir(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_all(&entry.path(), &dst.join(entry.file_name()))?;
        }
        fs::set_permissions(dst, meta.permissions())?;
        Ok(())
    } else {
        let mut input = fs::File::open(src)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dst)?;
        io::copy(&mut input, &mut output)?;
        fs::set_permissions(dst, meta.permissions())?;
        Ok(())
    }
}

/// 先在目标目录内生成完整临时副本，再切换为最终名称，避免复制失败留下半成品。
/// overwrite 先把旧目标同目录改名为备份；新目标就位后才回收备份。
/// 这样最终 rename 失败时可以原位恢复旧目标，而不是让失败操作留下空缺。
struct ExportCopyOutcome {
    warning: Option<String>,
}

fn copy_for_export(
    src: &Path,
    target: &Path,
    dest: &Path,
    overwrite: bool,
    temp_used: &mut HashSet<String>,
) -> Result<ExportCopyOutcome, String> {
    let temp_name = format!(".floepod-export-{}-{}", std::process::id(), db::now_ms());
    let temp = unique_target(dest, &temp_name, temp_used)?;
    if let Err(e) = copy_all(src, &temp) {
        let cleanup = remove_created_path(&temp).err();
        return Err(match cleanup {
            Some(cleanup) => format!("复制临时副本失败: {e}；清理失败: {cleanup}"),
            None => format!("复制临时副本失败: {e}"),
        });
    }

    let backup = match fs::symlink_metadata(target) {
        Ok(meta) => {
            if is_reparse_or_symlink(&meta) {
                let _ = remove_created_path(&temp);
                return Err("目标名称指向符号链接或目录重解析点".into());
            }
            if !overwrite {
                let _ = remove_created_path(&temp);
                return Err("目标在导出过程中已出现，请重新选择冲突策略".into());
            }
            let backup_name = format!(
                ".floepod-overwrite-backup-{}-{}",
                std::process::id(),
                db::now_ms()
            );
            let backup = match unique_target(dest, &backup_name, temp_used) {
                Ok(backup) => backup,
                Err(e) => {
                    let cleanup = remove_created_path(&temp).err();
                    return Err(match cleanup {
                        Some(cleanup) => {
                            format!("无法为旧目标分配备份名: {e}；清理临时副本失败: {cleanup}")
                        }
                        None => format!("无法为旧目标分配备份名: {e}"),
                    });
                }
            };
            if let Err(e) = fs::rename(target, &backup) {
                let cleanup = remove_created_path(&temp).err();
                return Err(match cleanup {
                    Some(cleanup) => {
                        format!("旧目标无法暂存为同目录备份: {e}；清理临时副本失败: {cleanup}")
                    }
                    None => format!("旧目标无法暂存为同目录备份: {e}"),
                });
            }
            Some(backup)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            let cleanup = remove_created_path(&temp).err();
            return Err(match cleanup {
                Some(cleanup) => format!("无法检查目标路径: {e}；清理临时副本失败: {cleanup}"),
                None => format!("无法检查目标路径: {e}"),
            });
        }
    };

    if let Err(e) = fs::rename(&temp, target) {
        let cleanup = remove_created_path(&temp).err();
        let restore = backup
            .as_ref()
            .and_then(|backup| match fs::symlink_metadata(target) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::rename(backup, target).err().map(|restore| {
                        format!("恢复旧目标失败: {restore}；备份保留于 {}", backup.display())
                    })
                }
                Ok(_) => Some(format!(
                    "目标名称被其他程序占用；旧目标备份保留于 {}",
                    backup.display()
                )),
                Err(error) => Some(format!(
                    "无法检查目标以恢复旧文件: {error}；备份保留于 {}",
                    backup.display()
                )),
            });
        let mut details = vec![format!("最终写入失败: {e}")];
        if let Some(cleanup) = cleanup {
            details.push(format!("清理临时副本失败: {cleanup}"));
        }
        if let Some(restore) = restore {
            details.push(restore);
        }
        return Err(details.join("；"));
    }

    let warning = backup.and_then(|backup| {
        trash::delete(&backup).err().map(|e| {
            format!(
                "新目标已写入，但旧目标备份无法移入回收站: {e}；备份保留于 {}",
                backup.display()
            )
        })
    });
    Ok(ExportCopyOutcome { warning })
}

fn resolved_path(path: &Path) -> Result<PathBuf, String> {
    settings::resolve_path(path)
}

fn ensure_copy_relation(src: &Path, target: &Path) -> Result<(), String> {
    let src_resolved = resolved_path(src)?;
    let target_resolved = resolved_path(target)?;
    if settings::paths_equal(&src_resolved, &target_resolved) {
        return Err(format!("源和目标不能相同: {}", src.display()));
    }
    if fs::symlink_metadata(src)
        .map(|m| m.is_dir())
        .unwrap_or(false)
        && settings::path_is_within(&target_resolved, &src_resolved)
    {
        return Err(format!(
            "不能把文件夹复制或移动到它自己的子目录: {}",
            target.display()
        ));
    }
    Ok(())
}

fn item_is_in_current_pod(item: &StagedItem, settings: &Settings) -> Result<PathBuf, String> {
    let pod = settings
        .pods
        .iter()
        .find(|p| p.id == item.pod_id as u64)
        .ok_or_else(|| format!("条目「{}」所属的匣已不存在", item.name))?;
    let root = resolved_path(Path::new(&pod.staging_folder))?;
    let raw = PathBuf::from(&item.staging_path);
    let name = raw
        .file_name()
        .ok_or_else(|| format!("条目「{}」的路径无效", item.name))?;
    let parent = raw
        .parent()
        .ok_or_else(|| format!("条目「{}」的路径无效", item.name))?;
    // 只解析父目录，不能先 canonicalize 整个条目：如果叶子本身是 symlink，
    // canonicalize 后再 trash 会删除链接目标而不是链接目录项。
    let path = resolved_path(parent)?.join(name);
    if !settings::path_is_within(&path, &root) || settings::paths_equal(&path, &root) {
        return Err(format!("条目「{}」已不在当前匣的暂存目录内", item.name));
    }
    match fs::symlink_metadata(&raw) {
        Ok(meta) if is_reparse_or_symlink(&meta) => {
            return Err(format!("条目「{}」是符号链接或目录重解析点", item.name));
        }
        Ok(_) => {
            let resolved = resolved_path(&raw)?;
            if !settings::paths_equal(&resolved, &path) {
                return Err(format!("条目「{}」的路径解析结果不一致", item.name));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("无法检查条目「{}」: {e}", item.name)),
    }
    Ok(path)
}

fn validated_settings(state: &AppState, pod_id: u64) -> Result<Settings, String> {
    let settings = load_settings(state)?;
    settings::validate(&settings, &data_dir_str(state))?;
    settings::validate_pod_for_io(&settings, &data_dir_str(state), pod_id)?;
    Ok(settings)
}

fn validate_item_pods(
    settings: &Settings,
    state: &AppState,
    items: &[StagedItem],
) -> Result<(), String> {
    let pod_ids: HashSet<u64> = items.iter().map(|item| item.pod_id as u64).collect();
    for pod_id in pod_ids {
        settings::validate_pod_for_io(settings, &data_dir_str(state), pod_id)?;
    }
    Ok(())
}

fn indexed_target_keys(state: &AppState, pod_id: u64) -> Result<HashSet<String>, String> {
    let conn = state.db.lock().unwrap();
    Ok(db::items_of_pod(&conn, pod_id as i64)?
        .into_iter()
        .map(|item| settings::path_key(Path::new(&item.staging_path)))
        .collect())
}

fn remove_created_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !is_reparse_or_symlink(&meta) => {
            fs::remove_dir_all(path).map_err(|e| e.to_string())
        }
        Ok(_) => fs::remove_file(path).map_err(|e| e.to_string()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[derive(Debug)]
struct StagedMove {
    staged: PathBuf,
    original: PathBuf,
    /// Cross-volume moves retain the source under this sibling name until the
    /// SQLite commit succeeds. This makes every pre-commit failure reversible
    /// with a same-volume rename instead of a second fallible cross-volume copy.
    quarantine: Option<PathBuf>,
}

fn internal_operation_path(parent: &Path, label: &str) -> Result<PathBuf, String> {
    for _ in 0..1024 {
        let sequence = FILE_OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".floepod-{label}-{}-{sequence:016x}",
            std::process::id()
        ));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Err(error) => {
                return Err(format!(
                    "无法检查内部临时路径 {}: {error}",
                    candidate.display()
                ));
            }
            Ok(_) => {}
        }
    }
    Err("无法分配内部临时路径".into())
}

fn restore_quarantined_move(record: &StagedMove) -> Result<(), String> {
    let quarantine = record
        .quarantine
        .as_ref()
        .ok_or_else(|| "缺少跨盘移动恢复路径".to_string())?;
    match fs::symlink_metadata(&record.original) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::rename(quarantine, &record.original).map_err(|error| {
                format!("恢复源路径 {} 失败: {error}", record.original.display())
            })?;
        }
        Ok(_) => return Err("原路径已被占用，未覆盖恢复".into()),
        Err(error) => return Err(format!("无法检查原路径: {error}")),
    }
    remove_created_path(&record.staged)
        .map_err(|error| format!("源已恢复，但暂存副本清理失败: {error}"))
}

fn rollback_staged_moves(records: &[StagedMove]) -> Vec<String> {
    records
        .iter()
        .rev()
        .filter_map(|record| {
            let result = if record.quarantine.is_some() {
                restore_quarantined_move(record)
            } else {
                restore_moved_path(&record.staged, &record.original)
            };
            result.err().map(|error| {
                format!(
                    "{} -> {}: {error}",
                    record.staged.display(),
                    record.original.display()
                )
            })
        })
        .collect()
}

/// Publish a move into staging without ever exposing a completed-but-untracked
/// cross-volume copy as a normal failure. The source is first renamed to a
/// sibling quarantine on its own volume, the copy is built under an internal
/// staging name, and only a complete copy is renamed to the final target.
fn move_into_staging(src: &Path, target: &Path) -> Result<StagedMove, String> {
    match fs::rename(src, target) {
        Ok(()) => Ok(StagedMove {
            staged: target.to_path_buf(),
            original: src.to_path_buf(),
            quarantine: None,
        }),
        Err(direct_error) => {
            let source_parent = src
                .parent()
                .ok_or_else(|| format!("源路径没有父目录: {}", src.display()))?;
            let quarantine = internal_operation_path(source_parent, "move-source")?;
            let target_parent = target
                .parent()
                .ok_or_else(|| format!("目标路径没有父目录: {}", target.display()))?;
            let temporary = internal_operation_path(target_parent, "inflight")?;
            fs::rename(src, &quarantine).map_err(|error| {
                format!("无法锁定跨盘移动源（直接移动错误: {direct_error}）：{error}")
            })?;

            let publish_result = (|| -> Result<(), String> {
                copy_all(&quarantine, &temporary)
                    .map_err(|error| format!("复制跨盘移动源失败: {error}"))?;
                fs::rename(&temporary, target)
                    .map_err(|error| format!("发布跨盘移动副本失败: {error}"))?;
                Ok(())
            })();

            if let Err(error) = publish_result {
                let mut rollback_errors = Vec::new();
                if let Err(cleanup) = remove_created_path(&temporary) {
                    rollback_errors.push(format!("清理临时副本失败: {cleanup}"));
                }
                if let Err(restore) = fs::rename(&quarantine, src) {
                    rollback_errors.push(format!("恢复源路径失败: {restore}"));
                }
                return Err(if rollback_errors.is_empty() {
                    error
                } else {
                    format!("{error}；{}", rollback_errors.join("；"))
                });
            }

            Ok(StagedMove {
                staged: target.to_path_buf(),
                original: src.to_path_buf(),
                quarantine: Some(quarantine),
            })
        }
    }
}

fn restore_moved_path(staged: &Path, original: &Path) -> Result<(), String> {
    match fs::symlink_metadata(original) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::rename(staged, original)
            .map_err(|e| e.to_string())
            .or_else(|_| {
                copy_all(staged, original).map_err(|e| e.to_string())?;
                remove_created_path(staged)
            }),
        Ok(_) => Err("原路径已被占用，未自动覆盖".into()),
        Err(e) => Err(format!("无法检查原路径: {e}")),
    }
}

fn sanitize_text_name(raw: &str) -> String {
    let bad = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let cleaned: String = raw
        .chars()
        .take(48)
        .map(|c| {
            if bad.contains(&c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "文字".to_string()
    } else {
        trimmed.to_string()
    }
}

fn text_file_base(title: Option<&str>, content: &str) -> String {
    let raw = title
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| content.lines().next().unwrap_or("文字"));
    let without_ext = if raw.to_ascii_lowercase().ends_with(".txt") {
        &raw[..raw.len().saturating_sub(4)]
    } else {
        raw
    };
    sanitize_text_name(without_ext)
}

fn pod_of_conn(conn: &rusqlite::Connection, state: &AppState, id: u64) -> Result<Pod, String> {
    let settings = load_settings_conn(conn, state)?;
    settings
        .pods
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "匣不存在".to_string())
}

fn staging_dir(pod: &Pod) -> Result<PathBuf, String> {
    let dir = PathBuf::from(&pod.staging_folder);
    fs::create_dir_all(&dir).map_err(|e| format!("暂存文件夹不可用: {e}"))?;
    Ok(dir)
}

fn staging_folder_changed(old: &str, new: &str) -> Result<bool, String> {
    let old = old.trim();
    let new = new.trim();
    match (old.is_empty(), new.is_empty()) {
        (true, true) => Ok(false),
        (true, false) | (false, true) => Ok(true),
        (false, false) => Ok(!settings::configured_paths_equal(
            Path::new(old),
            Path::new(new),
        )?),
    }
}

/* ---------- 启动信息 ---------- */

/// 调试日志：追加写入数据目录 debug.log（release 无控制台，用文件排查）。
pub fn debug_log(msg: &str) {
    use std::io::Write;
    let _guard = DEBUG_LOG_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dir = crate::paths::resolve();
    let _ = std::fs::create_dir_all(&dir);
    let log = dir.join("debug.log");
    if log.metadata().map(|m| m.len()).unwrap_or(0) >= DEBUG_LOG_MAX_BYTES {
        let rotated = dir.join("debug.log.1");
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(&log, &rotated);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
    {
        let bounded: String = msg.chars().take(DEBUG_LOG_MAX_MESSAGE_CHARS).collect();
        let _ = writeln!(f, "{}", bounded);
    }
    eprintln!("{msg}");
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    settings: Settings,
    monitors: Vec<serde_json::Value>,
    version: String,
}

#[tauri::command]
pub fn get_bootstrap(app: AppHandle) -> Result<Bootstrap, String> {
    let state = app.state::<AppState>();
    let settings = {
        let conn = state.db.lock().unwrap();
        load_settings_conn(&conn, &state)?
    };
    let monitors = manager::list_monitors(&app);
    Ok(Bootstrap {
        settings,
        monitors,
        version: VERSION.to_string(),
    })
}

#[tauri::command]
pub fn get_pod(app: AppHandle, pod_id: u64) -> Result<Option<Pod>, String> {
    Ok(load_settings(&app.state::<AppState>())?
        .pods
        .into_iter()
        .find(|p| p.id == pod_id))
}

#[tauri::command]
pub fn get_monitors(app: AppHandle) -> Vec<serde_json::Value> {
    manager::list_monitors(&app)
}

#[tauri::command]
pub fn get_modifier_state() -> crate::win::ModifierState {
    crate::win::modifier_state()
}

#[tauri::command]
pub fn get_hotkey_defaults() -> settings::Hotkeys {
    settings::Hotkeys::with_defaults()
}

/* ---------- 匣 CRUD ---------- */

/// 必须为 async 命令：apply_settings 会经 sync_pods 创建 WebView 窗口，
/// 同步命令在 Windows 上会与主线程消息循环互相等待而死锁（OOBE 创建匣卡死的根因）。
#[tauri::command]
pub async fn create_pod(
    app: AppHandle,
    config: serde_json::Value,
    reuse_existing: bool,
) -> Result<Pod, String> {
    let state = app.state::<AppState>();
    let _settings_operation = state.settings_ops.lock().unwrap();
    let pod = {
        let conn = state.db.lock().unwrap();
        let mut pod = pod_from_config(&config)?;
        // Creating a pod may have committed successfully even when the IPC response was lost
        // (notably during first-run setup).  Folder identity, rather than the spelling of the
        // Windows path, is the durable idempotency key because settings already forbid two pods
        // from sharing or nesting staging folders.
        let current = load_settings_conn(&conn, &state)?;
        let reusable = reuse_existing.then(|| {
            current.pods.iter().find(|existing| {
                !existing.staging_folder.is_empty()
                    && !pod.staging_folder.is_empty()
                    && matches!(
                        staging_folder_changed(&existing.staging_folder, &pod.staging_folder),
                        Ok(false)
                    )
            })
        });
        if let Some(existing) = reusable.flatten() {
            existing.clone()
        } else {
            let id = settings::next_pod_id(&conn, &data_dir_str(&state), VERSION)?;
            pod.id = id;
            settings::upsert_pod(&conn, &pod, &data_dir_str(&state), VERSION)?;
            pod
        }
    };
    manager::apply_settings(&app, &manager::current_settings(&app));
    // 定位了新文件夹：触发对账，把文件夹中已有的文件读入列表
    app.state::<AppState>()
        .watcher_dirty
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = app.emit(events::PODS_CHANGED, ());
    Ok(pod)
}

fn numeric_value(value: &serde_json::Value, field: &str) -> Result<f64, String> {
    let parsed = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
        .ok_or_else(|| format!("字段 {field} 必须是数字"))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(format!("字段 {field} 必须是有限数字"))
    }
}

fn unsigned_value(value: &serde_json::Value, field: &str) -> Result<u64, String> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
        .ok_or_else(|| format!("字段 {field} 必须是非负整数"))
}

fn string_value(value: &serde_json::Value, field: &str) -> Result<String, String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("字段 {field} 必须是字符串"))
}

/// 严格应用前端 Pod 补丁。缺省字段保留原值；已提供但类型错误或未知的字段必须报错，
/// 不能静默回退后再向前端报告“保存成功”。数字字符串仍兼容旧前端。
fn apply_pod_patch(pod: &mut Pod, patch: &serde_json::Value) -> Result<(), String> {
    let obj = patch
        .as_object()
        .ok_or_else(|| "匣配置补丁必须是对象".to_string())?;
    for (field, value) in obj {
        match field.as_str() {
            "name" => pod.name = string_value(value, field)?,
            "edge" => pod.edge = string_value(value, field)?,
            "monitor" => pod.monitor = string_value(value, field)?,
            "offset" => pod.offset = numeric_value(value, field)?,
            "stagingFolder" => pod.staging_folder = string_value(value, field)?,
            "opacity" => pod.opacity = numeric_value(value, field)?,
            "material" => pod.material = string_value(value, field)?,
            "panelWidth" => {
                pod.panel_width = u32::try_from(unsigned_value(value, field)?)
                    .map_err(|_| format!("字段 {field} 超出有效范围"))?;
            }
            "hoverDelayMs" => pod.hover_delay_ms = unsigned_value(value, field)?,
            "dropAction" => pod.drop_action = string_value(value, field)?,
            "enabled" => {
                pod.enabled = value
                    .as_bool()
                    .ok_or_else(|| format!("字段 {field} 必须是布尔值"))?;
            }
            _ => return Err(format!("未知匣配置字段: {field}")),
        }
    }
    Ok(())
}

/// 从前端配置构造 Pod；缺省采用安全默认值，显式值则严格解析。
fn pod_from_config(v: &serde_json::Value) -> Result<Pod, String> {
    let mut pod = Pod::default();
    apply_pod_patch(&mut pod, v)?;
    Ok(pod)
}

/// async 命令：与 create_pod 同理，可能触发窗口创建，须避开主线程。
#[tauri::command]
pub async fn update_pod(
    app: AppHandle,
    pod_id: u64,
    patch: serde_json::Value,
) -> Result<Pod, String> {
    let state = app.state::<AppState>();
    let _settings_operation = state.settings_ops.lock().unwrap();
    let file_operation = state.file_ops.lock().unwrap();
    let (pod, folder_changed, needs_reconcile) = {
        let mut conn = state.db.lock().unwrap();
        let mut pod = pod_of_conn(&conn, &state, pod_id)?;
        let old_folder = pod.staging_folder.clone();
        let old_enabled = pod.enabled;
        apply_pod_patch(&mut pod, &patch)?;
        let folder_changed = staging_folder_changed(&old_folder, &pod.staging_folder)?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        settings::upsert_pod(&tx, &pod, &data_dir_str(&state), VERSION)?;
        if folder_changed {
            // 更换目录的定义是“切换索引根”，不是迁移文件。旧目录中的物理文件保留，
            // 但旧索引必须与设置变更在同一事务中清除，不能等待 Watcher 偶然对账。
            db::delete_items_by_pod(&tx, pod_id as i64)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        let needs_reconcile = folder_changed || (!old_enabled && pod.enabled);
        (pod, folder_changed, needs_reconcile)
    };
    drop(file_operation);
    manager::apply_settings(&app, &manager::current_settings(&app));
    // 重新定位了暂存文件夹：触发对账，读取新文件夹中已有的文件
    if needs_reconcile {
        app.state::<AppState>()
            .watcher_dirty
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if folder_changed {
        emit_items_changed(&app, pod_id);
    }
    let _ = app.emit(events::PODS_CHANGED, ());
    Ok(pod)
}

/// async 命令：与 create_pod 同理，可能触发窗口创建，须避开主线程。
#[tauri::command]
pub async fn delete_pod(app: AppHandle, pod_id: u64, recycle_files: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _settings_operation = state.settings_ops.lock().unwrap();
    let file_operation = state.file_ops.lock().unwrap();
    let (settings_now, removed): (Settings, Vec<StagedItem>) = {
        let conn = state.db.lock().unwrap();
        let current = load_settings_conn(&conn, &state)?;
        let removed = db::items_of_pod(&conn, pod_id as i64)?;
        (current, removed)
    };

    if recycle_files {
        settings::validate(&settings_now, &data_dir_str(&state))?;
        settings::validate_pod_for_io(&settings_now, &data_dir_str(&state), pod_id)?;
        let validated: Vec<(&StagedItem, PathBuf)> = removed
            .iter()
            .map(|item| item_is_in_current_pod(item, &settings_now).map(|path| (item, path)))
            .collect::<Result<_, _>>()?;
        let mut deleted_ids = Vec::new();
        let mut failed = Vec::new();
        for (item, path) in validated {
            match fs::symlink_metadata(&path) {
                Ok(_) => match trash::delete(&path) {
                    Ok(()) => deleted_ids.push(item.id),
                    Err(e) => failed.push(format!("{}: {e}", item.name)),
                },
                Err(e) if e.kind() == io::ErrorKind::NotFound => deleted_ids.push(item.id),
                Err(e) => failed.push(format!("{}: {e}", item.name)),
            }
        }
        if !failed.is_empty() {
            if !deleted_ids.is_empty() {
                let mut conn = state.db.lock().unwrap();
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                db::delete_items_by_ids(&tx, &deleted_ids)?;
                tx.commit().map_err(|e| e.to_string())?;
                state.mark_staged();
                emit_items_changed(&app, pod_id);
            }
            return Err(format!("部分文件无法移入回收站：{}", failed.join("；")));
        }
    }

    // 无论是否保留物理文件，都必须删除索引；否则复用 pod_id 时旧条目会串入新匣。
    {
        let mut conn = state.db.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        db::delete_items_by_pod(&tx, pod_id as i64)?;
        settings::delete_pod(&tx, pod_id, &data_dir_str(&state), VERSION)?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    state.mark_staged();
    drop(file_operation);
    manager::apply_settings(&app, &manager::current_settings(&app));
    let _ = app.emit(events::PODS_CHANGED, ());
    Ok(())
}

/* ---------- 设置 ---------- */

/// async 命令：apply_settings 内可能创建窗口，同步执行会死锁主线程。
#[tauri::command]
pub async fn save_settings(app: AppHandle, patch: serde_json::Value) -> Result<Settings, String> {
    let state = app.state::<AppState>();
    let _settings_operation = state.settings_ops.lock().unwrap();
    // settings_ops 串行化整次事务；SQLite 锁只保护读取/写入本身，绝不能跨越
    // global-shortcut/autostart API。快捷键回调会持插件锁后读取 DB，反向持锁会死锁。
    let (prev, next) = {
        let conn = state.db.lock().unwrap();
        let prev = load_settings_conn(&conn, &state)?;
        let next = settings::merge_persist(&conn, patch, &data_dir_str(&state), VERSION)?;
        (prev, next)
    };
    let hotkeys_changed = next.hotkeys.toggle_bar != prev.hotkeys.toggle_bar
        || next.hotkeys.collect_clipboard != prev.hotkeys.collect_clipboard
        || next.hotkeys.open_panel != prev.hotkeys.open_panel;
    // 快捷键变更需可注册；失败则回滚热键字段并报错
    if hotkeys_changed {
        if let Err(e) = crate::hotkeys::register(&app, &next) {
            // register() 会先 unregister_all；注册新组合失败后必须恢复实际的旧注册，
            // 并将整个设置保存视为原子操作，不能只回滚数据库中的热键字段。
            let restore_error = crate::hotkeys::register(&app, &prev).err();
            let persist_error = {
                let conn = state.db.lock().unwrap();
                settings::persist(&conn, &prev).err()
            };
            if persist_error.is_none() {
                let _ = app.emit(events::SETTINGS_CHANGED, prev);
            }
            let mut errors = vec![e];
            if let Some(restore) = restore_error {
                errors.push(format!("恢复旧快捷键也失败：{restore}"));
            }
            if let Some(restore) = persist_error {
                errors.push(format!("恢复旧设置也失败：{restore}"));
            }
            return Err(errors.join("；"));
        }
    }
    // 自启动与快捷键一样是设置事务的一部分。系统 API 失败时恢复此前已经
    // 切换的外部状态和完整 DB 快照，不能向前端静默报告保存成功。
    if next.autostart != prev.autostart {
        if let Err(e) = manager::sync_autostart(&app, next.autostart) {
            let mut errors = vec![e];
            if let Err(restore) = manager::sync_autostart(&app, prev.autostart) {
                errors.push(format!("恢复旧自启动状态也失败：{restore}"));
            }
            if hotkeys_changed {
                if let Err(restore) = crate::hotkeys::register(&app, &prev) {
                    errors.push(format!("恢复旧快捷键也失败：{restore}"));
                }
            }
            let persist_result = {
                let conn = state.db.lock().unwrap();
                settings::persist(&conn, &prev)
            };
            match persist_result {
                Ok(()) => {
                    let _ = app.emit(events::SETTINGS_CHANGED, prev.clone());
                }
                Err(restore) => errors.push(format!("恢复旧设置也失败：{restore}")),
            }
            return Err(errors.join("；"));
        }
    }
    manager::apply_settings(&app, &next);
    Ok(next)
}

/* ---------- 暂存 ---------- */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageWarning {
    name: String,
    error: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagePathsResult {
    items: Vec<StagedItem>,
    warnings: Vec<StageWarning>,
}

#[tauri::command]
pub async fn stage_paths(
    app: AppHandle,
    pod_id: u64,
    paths: Vec<String>,
    action: String,
) -> Result<StagePathsResult, String> {
    run_blocking_command("文件暂存", move || {
        stage_paths_blocking(app, pod_id, paths, action)
    })
    .await
}

fn stage_paths_blocking(
    app: AppHandle,
    pod_id: u64,
    paths: Vec<String>,
    action: String,
) -> Result<StagePathsResult, String> {
    let state = app.state::<AppState>();
    let _file_operation = state.file_ops.lock().unwrap();
    if !matches!(action.as_str(), "copy" | "move" | "shortcut") {
        return Err(format!("未知动作: {action}"));
    }
    let settings_now = validated_settings(&state, pod_id)?;
    let pod = settings_now
        .pods
        .iter()
        .find(|p| p.id == pod_id)
        .cloned()
        .ok_or_else(|| "匣不存在".to_string())?;
    let dir = staging_dir(&pod)?;
    let dir_resolved = resolved_path(&dir)?;

    if paths.is_empty() {
        return Err("没有可暂存的文件".into());
    }
    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let source = PathBuf::from(path);
        if !source.is_absolute() {
            return Err(format!("源路径必须是绝对路径: {}", source.display()));
        }
        fs::symlink_metadata(&source)
            .map_err(|e| format!("无法读取源路径 {}: {e}", source.display()))?;
        sources.push(source);
    }

    // watcher 尚未对账的陈旧索引也必须占名；否则新文件写完后会在 UNIQUE 入库处失败，
    // 回滚又会删除刚写出的文件。
    let mut used = indexed_target_keys(&state, pod_id)?;
    let mut drafts: Vec<StagedItem> = Vec::new();
    let mut created_paths: Vec<PathBuf> = Vec::new();
    let mut staged_moves: Vec<StagedMove> = Vec::new();

    let prepare_result = (|| -> Result<(), String> {
        match action.as_str() {
            "shortcut" => {
                let mut pairs = Vec::new();
                for src in &sources {
                    let name = src
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "目标".into());
                    let target = unique_target(&dir, &lnk::shortcut_name_for(&name), &mut used)?;
                    pairs.push((src.clone(), target));
                }
                if let Err(e) = lnk::create_shortcuts(&pairs) {
                    for (_, target) in &pairs {
                        let _ = remove_created_path(target);
                    }
                    return Err(e);
                }
                created_paths.extend(pairs.iter().map(|(_, target)| target.clone()));
                for (src, target) in pairs {
                    if fs::symlink_metadata(&target).is_err() {
                        return Err(format!("快捷方式未生成: {}", target.display()));
                    }
                    let target_resolved = resolved_path(&target)?;
                    if !settings::path_is_within(&target_resolved, &dir_resolved) {
                        return Err("快捷方式目标路径越出暂存文件夹".into());
                    }
                    let name = target
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    drafts.push(StagedItem {
                        id: 0,
                        pod_id: pod.id as i64,
                        kind: "shortcut".into(),
                        staging_path: target_resolved.to_string_lossy().to_string(),
                        original_path: Some(resolved_path(&src)?.to_string_lossy().to_string()),
                        name,
                        ext: Some("lnk".into()),
                        size: 0,
                        created_at: db::now_ms(),
                    });
                }
            }
            act @ ("copy" | "move") => {
                for src in &sources {
                    let meta = fs::symlink_metadata(src)
                        .map_err(|e| format!("无法读取 {}: {e}", src.display()))?;
                    if is_reparse_or_symlink(&meta) {
                        return Err(format!("暂不支持符号链接或目录重解析点: {}", src.display()));
                    }
                    let source_resolved = resolved_path(src)?;
                    if source_resolved.parent().is_none() {
                        return Err(format!("不能暂存文件系统根目录: {}", src.display()));
                    }
                    let is_dir = meta.is_dir();
                    let original_path = source_resolved.to_string_lossy().to_string();
                    let name = src
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "未命名".into());
                    let target = unique_target(&dir, &name, &mut used)?;
                    ensure_copy_relation(src, &target)?;
                    if act == "move" {
                        let moved = move_into_staging(src, &target)
                            .map_err(|error| format!("移动 {name} 失败: {error}"))?;
                        staged_moves.push(moved);
                    } else {
                        if let Err(e) = copy_all(src, &target) {
                            let _ = remove_created_path(&target);
                            return Err(format!("复制 {name} 失败: {e}"));
                        }
                    }
                    created_paths.push(target.clone());
                    let target_resolved = resolved_path(&target)?;
                    if !settings::path_is_within(&target_resolved, &dir_resolved) {
                        return Err("暂存目标路径越出暂存文件夹".into());
                    }
                    let size = if is_dir {
                        0
                    } else {
                        fs::metadata(&target_resolved)
                            .map(|m| m.len() as i64)
                            .unwrap_or(0)
                    };
                    drafts.push(StagedItem {
                        id: 0,
                        pod_id: pod.id as i64,
                        kind: if is_dir { "folder" } else { "file" }.into(),
                        staging_path: target_resolved.to_string_lossy().to_string(),
                        original_path: Some(original_path),
                        name: target_resolved
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        ext: ext_of(&name),
                        size,
                        created_at: db::now_ms(),
                    });
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    })();
    if let Err(e) = prepare_result {
        let rollback_errors = if action == "move" {
            rollback_staged_moves(&staged_moves)
        } else {
            created_paths
                .iter()
                .filter_map(|path| {
                    remove_created_path(path)
                        .err()
                        .map(|err| format!("{}: {err}", path.display()))
                })
                .collect()
        };
        return Err(if rollback_errors.is_empty() {
            e
        } else {
            format!("{e}；回滚未完全成功：{}", rollback_errors.join("；"))
        });
    }

    // 文件 I/O 期间不占用全局 SQLite 锁；落库前再次确认匣没有被并发改到别处。
    let persist_result = (|| -> Result<Vec<StagedItem>, String> {
        let mut conn = state.db.lock().unwrap();
        let current = load_settings_conn(&conn, &state)?;
        settings::validate_pod_for_io(&current, &data_dir_str(&state), pod.id)?;
        let current_pod = current
            .pods
            .iter()
            .find(|p| p.id == pod.id)
            .ok_or_else(|| "暂存过程中匣已被删除".to_string())?;
        let current_dir = resolved_path(Path::new(&current_pod.staging_folder))?;
        if !settings::paths_equal(&current_dir, &dir_resolved) {
            return Err("暂存过程中匣的文件夹已改变".into());
        }
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut saved = Vec::with_capacity(drafts.len());
        for draft in &drafts {
            saved.push(db::insert_item(&tx, draft)?);
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(saved)
    })();
    let created = match persist_result {
        Ok(created) => created,
        Err(e) => {
            let rollback_errors = if action == "move" {
                rollback_staged_moves(&staged_moves)
            } else {
                created_paths
                    .iter()
                    .rev()
                    .filter_map(|path| {
                        remove_created_path(path)
                            .err()
                            .map(|error| format!("{}: {error}", path.display()))
                    })
                    .collect()
            };
            return Err(if rollback_errors.is_empty() {
                e
            } else {
                format!("{e}；回滚未完全成功：{}", rollback_errors.join("；"))
            });
        }
    };
    let mut warnings = Vec::new();
    for record in &staged_moves {
        let Some(quarantine) = record.quarantine.as_ref() else {
            continue;
        };
        if let Err(error) = remove_created_path(quarantine) {
            let name = record
                .original
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| record.original.display().to_string());
            warnings.push(StageWarning {
                name,
                error: format!(
                    "目标已暂存，但源卷临时副本 {} 清理失败: {error}",
                    quarantine.display()
                ),
            });
        }
    }
    state.mark_staged();
    emit_items_changed(&app, pod.id);
    Ok(StagePathsResult {
        items: created,
        warnings,
    })
}

#[tauri::command]
pub async fn stage_text(
    app: AppHandle,
    pod_id: u64,
    content: String,
    title: Option<String>,
) -> Result<StagedItem, String> {
    run_blocking_command("文字暂存", move || {
        stage_text_blocking(app, pod_id, content, title)
    })
    .await
}

fn stage_text_blocking(
    app: AppHandle,
    pod_id: u64,
    content: String,
    title: Option<String>,
) -> Result<StagedItem, String> {
    if content.trim().is_empty() {
        return Err("内容为空".into());
    }
    let state = app.state::<AppState>();
    let _file_operation = state.file_ops.lock().unwrap();
    let settings_now = validated_settings(&state, pod_id)?;
    let pod = settings_now
        .pods
        .iter()
        .find(|p| p.id == pod_id)
        .cloned()
        .ok_or_else(|| "匣不存在".to_string())?;
    let dir = staging_dir(&pod)?;
    let dir_resolved = resolved_path(&dir)?;

    let base = text_file_base(title.as_deref(), &content);
    let mut used = indexed_target_keys(&state, pod_id)?;
    let target = unique_target(&dir, &format!("{base}.txt"), &mut used)?;
    let size = content.len() as i64;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|e| format!("创建文字文件失败: {e}"))?;
    if let Err(e) = std::io::Write::write_all(&mut output, content.as_bytes()) {
        drop(output);
        let cleanup = remove_created_path(&target).err();
        return Err(match cleanup {
            Some(cleanup) => format!("写入失败: {e}；清理半成品失败: {cleanup}"),
            None => format!("写入失败: {e}"),
        });
    }
    drop(output);
    let target = match resolved_path(&target) {
        Ok(target) => target,
        Err(e) => {
            let cleanup = remove_created_path(&target).err();
            return Err(match cleanup {
                Some(cleanup) => format!("{e}；清理未入库文字文件失败: {cleanup}"),
                None => e,
            });
        }
    };
    if !settings::path_is_within(&target, &dir_resolved) {
        let cleanup = remove_created_path(&target).err();
        return Err(match cleanup {
            Some(cleanup) => format!("文字暂存目标越出暂存文件夹；清理失败: {cleanup}"),
            None => "文字暂存目标越出暂存文件夹".into(),
        });
    }

    let draft = StagedItem {
        id: 0,
        pod_id: pod.id as i64,
        kind: "text".into(),
        staging_path: target.to_string_lossy().to_string(),
        original_path: None,
        name: target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        ext: Some("txt".into()),
        size,
        created_at: db::now_ms(),
    };
    let persist_result = (|| -> Result<StagedItem, String> {
        let conn = state.db.lock().unwrap();
        let current = load_settings_conn(&conn, &state)?;
        settings::validate_pod_for_io(&current, &data_dir_str(&state), pod.id)?;
        let current_pod = current
            .pods
            .iter()
            .find(|p| p.id == pod.id)
            .ok_or_else(|| "文字写入过程中匣已被删除".to_string())?;
        let current_dir = resolved_path(Path::new(&current_pod.staging_folder))?;
        if !settings::paths_equal(&current_dir, &dir_resolved) {
            return Err("文字写入过程中匣的文件夹已改变".into());
        }
        db::insert_item(&conn, &draft)
    })();
    let item = match persist_result {
        Ok(item) => item,
        Err(e) => {
            let rollback = remove_created_path(&target).err();
            return Err(match rollback {
                Some(rollback) => format!("{e}；清理未入库文字文件失败: {rollback}"),
                None => e,
            });
        }
    };
    state.mark_staged();
    emit_items_changed(&app, pod.id);
    Ok(item)
}

#[tauri::command]
pub fn list_pod_items(app: AppHandle, pod_id: u64) -> Result<Vec<StagedItem>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    db::items_of_pod(&conn, pod_id as i64)
}

#[tauri::command]
pub async fn remove_items(app: AppHandle, ids: Vec<i64>, delete_files: bool) -> Result<(), String> {
    run_blocking_command("移出暂存项目", move || {
        remove_items_blocking(app, ids, delete_files)
    })
    .await
}

fn remove_items_blocking(app: AppHandle, ids: Vec<i64>, delete_files: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _file_operation = state.file_ops.lock().unwrap();
    let (settings_now, items) = {
        let conn = state.db.lock().unwrap();
        let items = db::items_by_ids(&conn, &ids)?;
        (load_settings_conn(&conn, &state)?, items)
    };
    let validated: Vec<(&StagedItem, PathBuf)> = if delete_files {
        settings::validate(&settings_now, &data_dir_str(&state))?;
        validate_item_pods(&settings_now, &state, &items)?;
        items
            .iter()
            .map(|item| item_is_in_current_pod(item, &settings_now).map(|path| (item, path)))
            .collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };

    let mut deleted_ids = Vec::new();
    let mut failed = Vec::new();
    if delete_files {
        for (item, path) in validated {
            match fs::symlink_metadata(&path) {
                Ok(_) => match trash::delete(&path) {
                    Ok(()) => deleted_ids.push(item.id),
                    Err(e) => failed.push(format!("{}: {e}", item.name)),
                },
                Err(e) if e.kind() == io::ErrorKind::NotFound => deleted_ids.push(item.id),
                Err(e) => failed.push(format!("{}: {e}", item.name)),
            }
        }
    } else {
        deleted_ids.extend(items.iter().map(|item| item.id));
    }

    let pod_ids: Vec<i64> = items
        .iter()
        .filter(|item| deleted_ids.contains(&item.id))
        .map(|item| item.pod_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if !deleted_ids.is_empty() {
        let mut conn = state.db.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        db::delete_items_by_ids(&tx, &deleted_ids)?;
        tx.commit().map_err(|e| e.to_string())?;
        state.mark_staged();
    }
    for pid in pod_ids {
        emit_items_changed(&app, pid as u64);
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("部分文件无法移入回收站：{}", failed.join("；")))
    }
}

/// 在启动 OLE Move 前捕获数据库归属与文件身份。返回的令牌一次有效且会过期。
#[tauri::command]
pub async fn prepare_drag_cut(
    app: AppHandle,
    pod_id: u64,
    paths: Vec<String>,
) -> Result<String, String> {
    run_blocking_command("准备剪切拖出", move || {
        prepare_drag_cut_blocking(app, pod_id, paths)
    })
    .await
}

fn prepare_drag_cut_blocking(
    app: AppHandle,
    pod_id: u64,
    paths: Vec<String>,
) -> Result<String, String> {
    if paths.is_empty() {
        return Err("没有可剪切拖出的项目".into());
    }
    let state = app.state::<AppState>();
    let _file_operation = state.file_ops.lock().unwrap();
    let (settings_now, items) = {
        let conn = state.db.lock().unwrap();
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for path in &paths {
            let key = settings::path_key(Path::new(path));
            if !seen.insert(key) {
                return Err(format!("剪切列表包含重复路径: {path}"));
            }
            let item = db::find_by_path(&conn, path)?
                .ok_or_else(|| format!("拒绝拖出不属于暂存列表的路径: {path}"))?;
            if item.pod_id != pod_id as i64 {
                return Err(format!("条目「{}」不属于当前匣", item.name));
            }
            found.push(item);
        }
        (load_settings_conn(&conn, &state)?, found)
    };

    settings::validate(&settings_now, &data_dir_str(&state))?;
    settings::validate_pod_for_io(&settings_now, &data_dir_str(&state), pod_id)?;
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        let path = item_is_in_current_pod(&item, &settings_now)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("无法读取剪切源「{}」: {error}", item.name))?;
        if is_reparse_or_symlink(&metadata) {
            return Err(format!("条目「{}」是符号链接或目录重解析点", item.name));
        }
        let identity = drag_cut_identity(&path, &metadata)?;
        entries.push(DragCutEntry {
            item_id: item.id,
            pod_id: item.pod_id,
            name: item.name,
            path,
            identity,
        });
    }
    Ok(store_drag_cut_snapshot(&state, entries))
}

/// 剪切拖出后的源清理：令牌先消费，再按当前 DB 归属和文件身份逐项复核。
#[tauri::command]
pub async fn finalize_drag_cut(app: AppHandle, token: String) -> Result<(), String> {
    run_blocking_command("剪切源清理", move || {
        finalize_drag_cut_blocking(app, token)
    })
    .await
}

fn finalize_drag_cut_blocking(app: AppHandle, token: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _file_operation = state.file_ops.lock().unwrap();
    // 一次性语义：无论后续验证或回收站操作是否成功，令牌都不能再次使用。
    let snapshot = take_drag_cut_snapshot(&state, &token)?;
    let item_ids: Vec<i64> = snapshot.entries.iter().map(|entry| entry.item_id).collect();
    let (settings_now, current_items) = {
        let conn = state.db.lock().unwrap();
        (
            load_settings_conn(&conn, &state)?,
            db::items_by_ids(&conn, &item_ids)?,
        )
    };
    settings::validate(&settings_now, &data_dir_str(&state))?;
    let mut by_id: HashMap<i64, StagedItem> = current_items
        .into_iter()
        .map(|item| (item.id, item))
        .collect();

    // 先完成当前索引/匣归属校验，再开始任何删除；单项失效只保护该项，不阻止
    // 其他仍与 prepare 快照一致的条目完成剪切。
    let mut candidates = Vec::new();
    let mut failed = Vec::new();
    for entry in snapshot.entries {
        let Some(item) = by_id.remove(&entry.item_id) else {
            failed.push(format!("{}: 暂存索引已改变，拒绝删除路径", entry.name));
            continue;
        };
        if item.pod_id != entry.pod_id {
            failed.push(format!("{}: 所属匣已改变，拒绝删除", entry.name));
            continue;
        }
        if let Err(error) =
            settings::validate_pod_for_io(&settings_now, &data_dir_str(&state), item.pod_id as u64)
        {
            failed.push(format!("{}: {error}", entry.name));
            continue;
        }
        let current_path = match item_is_in_current_pod(&item, &settings_now) {
            Ok(path) => path,
            Err(error) => {
                failed.push(format!("{}: {error}", entry.name));
                continue;
            }
        };
        if !settings::paths_equal(&current_path, &entry.path) {
            failed.push(format!("{}: 暂存路径已改变，拒绝删除", entry.name));
            continue;
        }
        candidates.push((entry, current_path));
    }

    let mut deleted_ids = Vec::new();
    let mut deleted_pods = HashSet::new();
    for (entry, path) in candidates {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if is_reparse_or_symlink(&metadata) => failed.push(format!(
                "{}: 拖拽期间已被替换为链接或重解析点，拒绝删除",
                entry.name
            )),
            Ok(metadata) => match drag_cut_identity(&path, &metadata) {
                Ok(current) if entry.identity.matches(&current) => match trash::delete(&path) {
                    Ok(()) => {
                        deleted_ids.push(entry.item_id);
                        deleted_pods.insert(entry.pod_id);
                    }
                    Err(error) => failed.push(format!("{}: {error}", entry.name)),
                },
                Ok(_) => failed.push(format!(
                    "{}: 拖拽期间文件或目录内容已被替换或修改，拒绝删除",
                    entry.name
                )),
                Err(error) => failed.push(format!(
                    "{}: 无法复核目录内容，拒绝删除: {error}",
                    entry.name
                )),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                deleted_ids.push(entry.item_id);
                deleted_pods.insert(entry.pod_id);
            }
            Err(error) => failed.push(format!("{}: {error}", entry.name)),
        }
    }
    if !deleted_ids.is_empty() {
        let mut conn = state.db.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        db::delete_items_by_ids(&tx, &deleted_ids)?;
        tx.commit().map_err(|e| e.to_string())?;
        state.mark_staged();
    }
    for pid in deleted_pods {
        emit_items_changed(&app, pid as u64);
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("部分剪切源无法清理：{}", failed.join("；")))
    }
}

/// 主动撤销尚未消费的剪切令牌。幂等，便于前端在 finally 中无条件清理。
#[tauri::command]
pub fn cancel_drag_cut(app: AppHandle, token: String) {
    let state = app.state::<AppState>();
    let now = Instant::now();
    let mut snapshots = state.drag_cut_tokens.lock().unwrap();
    snapshots.remove(&token);
    snapshots.retain(|_, snapshot| snapshot.expires_at > now);
}

/* ---------- 导出 ---------- */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportIssue {
    id: i64,
    name: String,
    error: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    conflicts: Vec<String>,
    completed_ids: Vec<i64>,
    skipped_ids: Vec<i64>,
    stale_ids: Vec<i64>,
    failed: Vec<ExportIssue>,
    warnings: Vec<ExportIssue>,
}

#[tauri::command]
pub async fn export_items(
    app: AppHandle,
    ids: Vec<i64>,
    dest_dir: String,
    mode: String,
    on_conflict: String,
) -> Result<ExportResult, String> {
    run_blocking_command("导出项目", move || {
        export_items_blocking(app, ids, dest_dir, mode, on_conflict)
    })
    .await
}

fn export_items_blocking(
    app: AppHandle,
    ids: Vec<i64>,
    dest_dir: String,
    mode: String,
    on_conflict: String,
) -> Result<ExportResult, String> {
    if !matches!(mode.as_str(), "copy" | "move") {
        return Err(format!("未知导出模式: {mode}"));
    }
    if !matches!(
        on_conflict.as_str(),
        "ask" | "overwrite" | "skip" | "rename"
    ) {
        return Err(format!("未知冲突策略: {on_conflict}"));
    }
    let state = app.state::<AppState>();
    let _file_operation = state.file_ops.lock().unwrap();
    let (settings_now, items) = {
        let conn = state.db.lock().unwrap();
        (
            load_settings_conn(&conn, &state)?,
            db::items_by_ids(&conn, &ids)?,
        )
    };
    if items.is_empty() {
        return Ok(ExportResult::default());
    }
    settings::validate(&settings_now, &data_dir_str(&state))?;
    validate_item_pods(&settings_now, &state, &items)?;
    let sources: Vec<(&StagedItem, PathBuf)> = items
        .iter()
        .map(|item| item_is_in_current_pod(item, &settings_now).map(|path| (item, path)))
        .collect::<Result<_, _>>()?;

    let dest = PathBuf::from(&dest_dir);
    if !dest.is_absolute() {
        return Err("目标文件夹必须是绝对路径".into());
    }
    fs::create_dir_all(&dest).map_err(|e| format!("目标文件夹不可用: {e}"))?;
    let dest = resolved_path(&dest)?;

    let mut conflict_keys = HashSet::new();
    let mut conflicts = Vec::new();
    for (item, _) in &sources {
        let candidate = dest.join(&item.name);
        let key = settings::path_key(&resolved_path(&candidate)?);
        if fs::symlink_metadata(&candidate).is_ok() || !conflict_keys.insert(key) {
            conflicts.push(item.name.clone());
        }
    }
    if on_conflict == "ask" && !conflicts.is_empty() {
        return Ok(ExportResult {
            conflicts,
            ..ExportResult::default()
        });
    }

    let mut used: HashSet<String> = HashSet::new();
    let mut temp_used: HashSet<String> = HashSet::new();
    let mut moved_ids = Vec::new();
    let mut changed_pods = HashSet::new();
    let mut result = ExportResult::default();
    let mut batch_targets = HashSet::new();
    for (item, src) in sources {
        let issue = |error: String| ExportIssue {
            id: item.id,
            name: item.name.clone(),
            error,
        };
        match fs::symlink_metadata(&src) {
            Ok(meta) if is_reparse_or_symlink(&meta) => {
                result
                    .failed
                    .push(issue("不支持符号链接或目录重解析点".into()));
                continue;
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if mode == "move" {
                    moved_ids.push(item.id);
                    changed_pods.insert(item.pod_id);
                    // The stale index is cleaned up, but no file was produced in the target.
                    // Keep it out of completed_ids so the UI does not claim it was moved.
                    result.stale_ids.push(item.id);
                } else {
                    result.failed.push(issue("源文件已不存在".into()));
                }
                continue;
            }
            Err(e) => {
                result.failed.push(issue(e.to_string()));
                continue;
            }
        }

        let target = match on_conflict.as_str() {
            "overwrite" => dest.join(&item.name),
            "skip" => {
                if fs::symlink_metadata(dest.join(&item.name)).is_ok() {
                    // 跳过表示源仍留在暂存区，绝不能删除其数据库记录。
                    result.skipped_ids.push(item.id);
                    continue;
                }
                dest.join(&item.name)
            }
            "rename" => match unique_target(&dest, &item.name, &mut used) {
                Ok(target) => target,
                Err(e) => {
                    result.failed.push(issue(e));
                    continue;
                }
            },
            "ask" => {
                // 没有冲突时 ask 与普通目标名相同。
                dest.join(&item.name)
            }
            _ => unreachable!(),
        };
        if let Ok(meta) = fs::symlink_metadata(&target) {
            if is_reparse_or_symlink(&meta) {
                result
                    .failed
                    .push(issue("目标名称指向符号链接或目录重解析点".into()));
                continue;
            }
        }
        let target_resolved = match resolved_path(&target) {
            Ok(path) => path,
            Err(e) => {
                result.failed.push(issue(e));
                continue;
            }
        };
        if !settings::path_is_within(&target_resolved, &dest)
            || settings::paths_equal(&target_resolved, &dest)
        {
            result.failed.push(issue("目标路径越出所选目录".into()));
            continue;
        }
        if !batch_targets.insert(settings::path_key(&target_resolved)) {
            result.failed.push(issue("批次内存在重复目标名称".into()));
            continue;
        }
        if let Err(e) = ensure_copy_relation(&src, &target) {
            result.failed.push(issue(e));
            continue;
        }
        let copy_outcome = match copy_for_export(
            &src,
            &target,
            &dest,
            on_conflict == "overwrite",
            &mut temp_used,
        ) {
            Ok(outcome) => outcome,
            Err(e) => {
                result.failed.push(issue(format!("导出失败: {e}")));
                continue;
            }
        };
        result.completed_ids.push(item.id);
        if let Some(warning) = copy_outcome.warning {
            result.warnings.push(issue(warning));
        }
        if mode == "move" {
            match trash::delete(&src) {
                Ok(()) => {
                    moved_ids.push(item.id);
                    changed_pods.insert(item.pod_id);
                }
                Err(e) => {
                    // 目标副本已生成，但源仍在；保留数据库记录并明确向用户报告。
                    result
                        .warnings
                        .push(issue(format!("已复制，但源文件无法移入回收站: {e}")));
                }
            }
        }
    }

    if !moved_ids.is_empty() {
        let delete_result = (|| -> Result<(), String> {
            let mut conn = state.db.lock().unwrap();
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            db::delete_items_by_ids(&tx, &moved_ids)?;
            tx.commit().map_err(|e| e.to_string())
        })();
        match delete_result {
            Ok(()) => state.mark_staged(),
            Err(e) => {
                for id in &moved_ids {
                    let name = items
                        .iter()
                        .find(|item| item.id == *id)
                        .map(|item| item.name.clone())
                        .unwrap_or_else(|| format!("条目 {id}"));
                    result.warnings.push(ExportIssue {
                        id: *id,
                        name,
                        error: format!("文件已移动，但索引清理失败，将由 watcher 重试: {e}"),
                    });
                }
                state
                    .watcher_dirty
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    for pid in changed_pods {
        emit_items_changed(&app, pid as u64);
    }
    Ok(result)
}

/* ---------- 缩略图 ---------- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailPayload {
    mime: String,
    bytes: Vec<u8>,
}

const THUMB_MAX_SIDE: u32 = 256;
const THUMB_MAX_SRC: u64 = 64 * 1024 * 1024;
const THUMB_MAX_DIMENSION: u32 = 16_384;
const THUMB_MAX_ALLOC: u64 = 256 * 1024 * 1024;

#[tauri::command]
pub async fn read_thumbnail(
    app: AppHandle,
    path: String,
) -> Result<Option<ThumbnailPayload>, String> {
    run_blocking_command("读取缩略图", move || {
        read_thumbnail_blocking(app, path)
    })
    .await
}

fn read_thumbnail_blocking(
    app: AppHandle,
    path: String,
) -> Result<Option<ThumbnailPayload>, String> {
    let state = app.state::<AppState>();
    // Serialize only the DB/path validation and source snapshot with mutating file
    // operations. Image probing/decoding is CPU-heavy and must not hold `file_ops`,
    // otherwise a long list of thumbnails starves stage/export/remove and watcher.
    let bytes = {
        let _file_operation = state.file_ops.lock().unwrap();
        let (settings_now, item) = {
            let conn = state.db.lock().unwrap();
            (
                load_settings_conn(&conn, &state)?,
                db::find_by_path(&conn, &path)?,
            )
        };
        let Some(item) = item else {
            return Ok(None);
        };
        settings::validate(&settings_now, &data_dir_str(&state))?;
        settings::validate_pod_for_io(&settings_now, &data_dir_str(&state), item.pod_id as u64)?;
        let target = item_is_in_current_pod(&item, &settings_now)?;
        let ext = ext_of(
            &target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
        );
        let Some(ext) = ext else {
            return Ok(None);
        };
        if !matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico"
        ) {
            return Ok(None);
        }
        let meta = fs::metadata(&target).map_err(|e| e.to_string())?;
        if meta.len() > THUMB_MAX_SRC {
            return Ok(None);
        }
        let bytes = fs::read(&target).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > THUMB_MAX_SRC {
            return Ok(None);
        }
        bytes
    };

    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(THUMB_MAX_DIMENSION);
    limits.max_image_height = Some(THUMB_MAX_DIMENSION);
    limits.max_alloc = Some(THUMB_MAX_ALLOC);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?;
    let thumb = img.thumbnail(THUMB_MAX_SIDE, THUMB_MAX_SIDE);
    let mut png = Vec::new();
    image::DynamicImage::write_to(
        &thumb,
        std::io::Cursor::new(&mut png),
        image::ImageFormat::Png,
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(ThumbnailPayload {
        mime: "image/png".into(),
        bytes: png,
    }))
}

/* ---------- 窗口编排（按匣） ---------- */

fn emit_items_changed(app: &AppHandle, pod_id: u64) {
    let payload = serde_json::json!({ "podId": pod_id });
    if manager::pod_panel(app, pod_id).is_some() {
        let _ = app.emit_to(
            format!("pod_{pod_id}_panel"),
            events::ITEMS_CHANGED,
            payload.clone(),
        );
    }
    if manager::pod_bar(app, pod_id).is_some() {
        let _ = app.emit_to(format!("pod_{pod_id}"), events::ITEMS_CHANGED, payload);
    }
}

#[tauri::command]
pub async fn show_panel(app: AppHandle, pod_id: u64) {
    manager::show_panel(&app, pod_id);
}

#[tauri::command]
pub async fn toggle_panel(app: AppHandle, pod_id: u64) {
    manager::toggle_panel(&app, pod_id);
}

#[tauri::command]
pub async fn hide_panel(app: AppHandle, pod_id: u64) {
    manager::hide_panel(&app, pod_id);
}

#[tauri::command]
pub async fn set_panel_mode(app: AppHandle, pod_id: u64, mode: String) -> Result<(), String> {
    manager::set_panel_mode(&app, pod_id, &mode)
}

#[tauri::command]
pub async fn hold_pending_drop(
    app: AppHandle,
    pod_id: u64,
    paths: Vec<String>,
) -> Result<(), String> {
    manager::hold_pending_drop(&app, pod_id, paths)
}

#[tauri::command]
pub async fn report_presence(app: AppHandle, pod_id: u64, window: String, inside: bool) {
    manager::report_presence(&app, pod_id, &window, inside);
}

#[tauri::command]
pub async fn set_panel_pinned(app: AppHandle, pod_id: u64, pinned: bool) {
    manager::set_panel_pinned(&app, pod_id, pinned);
}

#[tauri::command]
pub async fn set_dragging_out(app: AppHandle, pod_id: u64, dragging: bool) {
    manager::set_dragging_out(&app, pod_id, dragging);
}

#[tauri::command]
pub async fn set_pod_accept(app: AppHandle, pod_id: u64, accepting: bool) {
    manager::set_pod_accept(&app, pod_id, accepting);
}

#[tauri::command]
pub async fn set_panel_size(app: AppHandle, pod_id: u64, _width: u32, height: u32) {
    manager::set_panel_size(&app, pod_id, height);
}

#[tauri::command]
pub async fn toggle_all_bars(app: AppHandle) {
    let visible = {
        let pod = manager::current_settings(&app)
            .pods
            .into_iter()
            .find(|p| p.enabled);
        match pod {
            Some(p) => manager::pod_bar(&app, p.id)
                .map(|b| b.is_visible().unwrap_or(false))
                .unwrap_or(false),
            None => false,
        }
    };
    manager::set_all_bars(&app, !visible);
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    manager::open_settings(&app);
}

/// 前端错误上报（写入数据目录 debug.log，用于排查）。
#[tauri::command]
pub fn log_frontend(msg: String) {
    debug_log(&format!("[frontend] {msg}"));
}

/// 前端生命周期日志（写入数据目录 debug.log，用于排查创建流程）。
#[tauri::command]
pub fn app_log(msg: String) {
    debug_log(&format!("[ui] {msg}"));
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/* ---------- 测试 ---------- */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_target_appends_number() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        fs::write(dir.join("a.pdf"), b"x").unwrap();
        let mut used = HashSet::new();
        let t1 = unique_target(dir, "a.pdf", &mut used).unwrap();
        assert_eq!(t1.file_name().unwrap().to_string_lossy(), "a (2).pdf");
        let t2 = unique_target(dir, "a.pdf", &mut used).unwrap();
        assert_eq!(t2.file_name().unwrap().to_string_lossy(), "a (3).pdf");
        let t3 = unique_target(dir, "b.txt", &mut used).unwrap();
        assert_eq!(t3.file_name().unwrap().to_string_lossy(), "b.txt");
    }

    #[test]
    fn copy_all_never_overwrites_an_existing_target() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source.txt");
        let target = tmp.path().join("target.txt");
        fs::write(&source, b"new").unwrap();
        fs::write(&target, b"keep").unwrap();

        assert!(copy_all(&source, &target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn drag_cut_identity_rejects_a_modified_source() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"old").unwrap();
        let before = drag_cut_identity(&source, &fs::symlink_metadata(&source).unwrap()).unwrap();
        let unchanged =
            drag_cut_identity(&source, &fs::symlink_metadata(&source).unwrap()).unwrap();
        assert!(before.matches(&unchanged));

        fs::write(&source, b"replacement with a different size").unwrap();
        let modified = drag_cut_identity(&source, &fs::symlink_metadata(&source).unwrap()).unwrap();
        assert!(!before.matches(&modified));
    }

    #[test]
    fn drag_cut_identity_rejects_a_modified_directory_descendant() {
        let tmp = tempfile::tempdir().unwrap();
        let directory = tmp.path().join("source");
        fs::create_dir(&directory).unwrap();
        let child = directory.join("child.txt");
        fs::write(&child, b"old").unwrap();
        let before =
            drag_cut_identity(&directory, &fs::symlink_metadata(&directory).unwrap()).unwrap();

        fs::write(&child, b"replacement with a different size").unwrap();
        let modified =
            drag_cut_identity(&directory, &fs::symlink_metadata(&directory).unwrap()).unwrap();
        assert!(!before.matches(&modified));
    }

    #[test]
    fn drag_cut_token_is_consumed_once() {
        let tmp = tempfile::tempdir().unwrap();
        let state = AppState::new(
            rusqlite::Connection::open_in_memory().unwrap(),
            tmp.path().to_path_buf(),
        );
        let token = store_drag_cut_snapshot(&state, Vec::new());
        assert!(take_drag_cut_snapshot(&state, &token).is_ok());
        assert!(take_drag_cut_snapshot(&state, &token).is_err());
    }

    #[test]
    fn equivalent_staging_folder_spelling_is_not_a_folder_change() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().to_string_lossy();
        let with_dot = tmp.path().join(".").to_string_lossy().into_owned();
        assert!(!staging_folder_changed(&plain, &with_dot).unwrap());
    }

    #[test]
    fn ext_of_handles_edge_cases() {
        assert_eq!(ext_of("a.PDF").as_deref(), Some("pdf"));
        assert_eq!(ext_of(".gitignore"), None);
        assert_eq!(ext_of("noext"), None);
        assert_eq!(ext_of("arch.tar.gz").as_deref(), Some("gz"));
    }

    #[test]
    fn sanitize_keeps_readable_head() {
        assert_eq!(sanitize_text_name("héllo world"), "héllo world");
        assert_eq!(sanitize_text_name("a<b>c:d"), "a b c d");
        assert_eq!(sanitize_text_name("   "), "文字");
        let long = "字".repeat(80);
        assert_eq!(sanitize_text_name(&long).chars().count(), 48);
    }

    #[test]
    fn text_title_is_optional_and_txt_suffix_is_not_duplicated() {
        assert_eq!(text_file_base(Some("实验记录"), "正文"), "实验记录");
        assert_eq!(text_file_base(Some("实验记录.txt"), "正文"), "实验记录");
        assert_eq!(text_file_base(Some("  "), "第一行\n第二行"), "第一行");
    }

    #[test]
    fn pod_from_config_accepts_numeric_strings() {
        // 前端 range 输入可能传出字符串数字，应宽容解析
        let v = serde_json::json!({
            "name": "我的匣",
            "edge": "left",
            "stagingFolder": "D:\\暂存",
            "opacity": "0.85",
            "panelWidth": "380",
        });
        let pod = pod_from_config(&v).unwrap();
        assert_eq!(pod.name, "我的匣");
        assert_eq!(pod.opacity, 0.85);
        assert_eq!(pod.panel_width, 380);
        assert!(pod.enabled);
    }

    #[test]
    fn pod_patch_rejects_unknown_or_mistyped_fields() {
        let mut pod = Pod::default();
        assert!(apply_pod_patch(&mut pod, &serde_json::json!({ "enabled": "yes" })).is_err());
        assert!(apply_pod_patch(&mut pod, &serde_json::json!({ "panelWidth": -1 })).is_err());
        assert!(apply_pod_patch(&mut pod, &serde_json::json!({ "typo": true })).is_err());
    }
}
