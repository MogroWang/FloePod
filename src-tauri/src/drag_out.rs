use std::collections::{hash_map::RandomState, HashMap, HashSet};
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::db::{self, StagedItem};
use crate::events;
use crate::file_ops;
use crate::settings;
use crate::staging;
use crate::state::{AppState, DragCutEntry, DragCutFileIdentity, DragCutSnapshot};

const TOKEN_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_TREE_ENTRIES: usize = 100_000;
const FNV64_OFFSET: u64 = 0xcbf29ce484222325;
const FNV64_PRIME: u64 = 0x100000001b3;

fn basic_identity(metadata: &fs::Metadata) -> DragCutFileIdentity {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        DragCutFileIdentity {
            // Stable Rust does not yet expose all by-handle IDs used here. The
            // optional fields allow adopting them without changing token semantics.
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
    let identity = basic_identity(metadata);
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
        if *count > MAX_TREE_ENTRIES {
            return Err(format!(
                "目录树超过 {MAX_TREE_ENTRIES} 项，无法安全剪切拖出"
            ));
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("无法读取目录项 {}: {error}", path.display()))?;
        if file_ops::is_reparse_or_symlink(&metadata) {
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

fn identity(path: &Path, metadata: &fs::Metadata) -> Result<DragCutFileIdentity, String> {
    let mut identity = basic_identity(metadata);
    if metadata.is_dir() {
        let mut hash = FNV64_OFFSET;
        let mut count = 0;
        fingerprint_directory(path, &mut hash, &mut count)?;
        fingerprint_u64(&mut hash, count as u64);
        identity.tree_fingerprint = Some(hash);
    }
    Ok(identity)
}

fn new_token(state: &AppState) -> String {
    let sequence = state
        .next_drag_cut_token
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(sequence);
    hasher.write(&timestamp.to_le_bytes());
    format!("cut-{sequence:016x}-{:016x}", hasher.finish())
}

fn store_snapshot(state: &AppState, entries: Vec<DragCutEntry>) -> String {
    let now = Instant::now();
    let token = new_token(state);
    let mut snapshots = state.drag_cut_tokens.lock().unwrap();
    snapshots.retain(|_, snapshot| snapshot.expires_at > now);
    snapshots.insert(
        token.clone(),
        DragCutSnapshot {
            expires_at: now + TOKEN_TTL,
            entries,
        },
    );
    token
}

fn take_snapshot(state: &AppState, token: &str) -> Result<DragCutSnapshot, String> {
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

pub fn prepare(app: AppHandle, pod_id: u64, paths: Vec<String>) -> Result<String, String> {
    if paths.is_empty() {
        return Err("没有可剪切拖出的项目".into());
    }
    let state = app.state::<AppState>();
    let _operation = state.file_ops.lock().unwrap();
    let (current, items) = {
        let connection = state.db.lock().unwrap();
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for path in &paths {
            let key = settings::path_key(Path::new(path));
            if !seen.insert(key) {
                return Err(format!("剪切列表包含重复路径: {path}"));
            }
            let item = db::find_by_path(&connection, path)?
                .ok_or_else(|| format!("拒绝拖出不属于暂存列表的路径: {path}"))?;
            if item.pod_id != pod_id as i64 {
                return Err(format!("条目「{}」不属于当前匣", item.name));
            }
            found.push(item);
        }
        (staging::load_settings_from(&connection, &state)?, found)
    };

    settings::validate(&current, &staging::data_dir(&state))?;
    settings::validate_pod_for_io(&current, &staging::data_dir(&state), pod_id)?;
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        let path = staging::item_path(&item, &current)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("无法读取剪切源「{}」: {error}", item.name))?;
        if file_ops::is_reparse_or_symlink(&metadata) {
            return Err(format!("条目「{}」是符号链接或目录重解析点", item.name));
        }
        let identity = identity(&path, &metadata)?;
        entries.push(DragCutEntry {
            item_id: item.id,
            pod_id: item.pod_id,
            name: item.name,
            path,
            identity,
        });
    }
    Ok(store_snapshot(&state, entries))
}

pub fn finalize(app: AppHandle, token: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _operation = state.file_ops.lock().unwrap();
    // Consume first: no validation or Recycle Bin failure can make a token reusable.
    let snapshot = take_snapshot(&state, &token)?;
    let ids: Vec<_> = snapshot.entries.iter().map(|entry| entry.item_id).collect();
    let (current, items) = {
        let connection = state.db.lock().unwrap();
        (
            staging::load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, &ids)?,
        )
    };
    settings::validate(&current, &staging::data_dir(&state))?;
    let mut items: HashMap<i64, StagedItem> =
        items.into_iter().map(|item| (item.id, item)).collect();

    let mut candidates = Vec::new();
    let mut failed = Vec::new();
    for entry in snapshot.entries {
        let Some(item) = items.remove(&entry.item_id) else {
            failed.push(format!("{}: 暂存索引已改变，拒绝删除路径", entry.name));
            continue;
        };
        if item.pod_id != entry.pod_id {
            failed.push(format!("{}: 所属匣已改变，拒绝删除", entry.name));
            continue;
        }
        if let Err(error) =
            settings::validate_pod_for_io(&current, &staging::data_dir(&state), item.pod_id as u64)
        {
            failed.push(format!("{}: {error}", entry.name));
            continue;
        }
        let path = match staging::item_path(&item, &current) {
            Ok(path) => path,
            Err(error) => {
                failed.push(format!("{}: {error}", entry.name));
                continue;
            }
        };
        if !settings::paths_equal(&path, &entry.path) {
            failed.push(format!("{}: 暂存路径已改变，拒绝删除", entry.name));
            continue;
        }
        candidates.push((entry, path));
    }

    let mut removed_ids = Vec::new();
    let mut changed_pods = HashSet::new();
    for (entry, path) in candidates {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if file_ops::is_reparse_or_symlink(&metadata) => failed.push(format!(
                "{}: 拖拽期间已被替换为链接或重解析点，拒绝删除",
                entry.name
            )),
            Ok(metadata) => match identity(&path, &metadata) {
                Ok(current) if entry.identity.matches(&current) => match trash::delete(&path) {
                    Ok(()) => {
                        removed_ids.push(entry.item_id);
                        changed_pods.insert(entry.pod_id);
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
                removed_ids.push(entry.item_id);
                changed_pods.insert(entry.pod_id);
            }
            Err(error) => failed.push(format!("{}: {error}", entry.name)),
        }
    }
    if !removed_ids.is_empty() {
        let mut connection = state.db.lock().unwrap();
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        db::delete_items_by_ids(&transaction, &removed_ids)?;
        transaction.commit().map_err(|error| error.to_string())?;
        state.mark_staged();
    }
    for pod_id in changed_pods {
        events::emit_items_changed(&app, pod_id as u64);
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("部分剪切源无法清理：{}", failed.join("；")))
    }
}

pub fn cancel(app: &AppHandle, token: &str) {
    let state = app.state::<AppState>();
    let now = Instant::now();
    let mut snapshots = state.drag_cut_tokens.lock().unwrap();
    snapshots.remove(token);
    snapshots.retain(|_, snapshot| snapshot.expires_at > now);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_rejects_modified_files_and_directory_descendants() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("source.txt");
        fs::write(&file, b"old").unwrap();
        let before = identity(&file, &fs::symlink_metadata(&file).unwrap()).unwrap();
        assert!(before.matches(&identity(&file, &fs::symlink_metadata(&file).unwrap()).unwrap()));
        fs::write(&file, b"replacement with a different size").unwrap();
        assert!(!before.matches(&identity(&file, &fs::symlink_metadata(&file).unwrap()).unwrap()));

        let directory = temporary.path().join("directory");
        fs::create_dir(&directory).unwrap();
        let child = directory.join("child.txt");
        fs::write(&child, b"old").unwrap();
        let before = identity(&directory, &fs::symlink_metadata(&directory).unwrap()).unwrap();
        fs::write(&child, b"replacement with a different size").unwrap();
        assert!(!before
            .matches(&identity(&directory, &fs::symlink_metadata(&directory).unwrap()).unwrap()));
    }

    #[test]
    fn token_is_single_use() {
        let temporary = tempfile::tempdir().unwrap();
        let state = AppState::new(
            rusqlite::Connection::open_in_memory().unwrap(),
            temporary.path().to_path_buf(),
        );
        let token = store_snapshot(&state, Vec::new());
        assert!(take_snapshot(&state, &token).is_ok());
        assert!(take_snapshot(&state, &token).is_err());
    }
}
