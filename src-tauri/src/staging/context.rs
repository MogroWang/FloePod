use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::db::{self, StagedItem};
use crate::file_ops::{self};
use crate::settings::{self, Pod, Settings};
use crate::state::AppState;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn data_dir(state: &AppState) -> String {
    state.data_dir.to_string_lossy().to_string()
}

pub fn load_settings(state: &AppState) -> Result<Settings, String> {
    let connection = state.db.lock().unwrap();
    load_settings_from(&connection, state)
}

pub fn load_settings_from(
    connection: &rusqlite::Connection,
    state: &AppState,
) -> Result<Settings, String> {
    settings::load(connection, &data_dir(state), VERSION)
}

pub fn validated_settings(state: &AppState, pod_id: u64) -> Result<Settings, String> {
    let current = load_settings(state)?;
    settings::validate(&current, &data_dir(state))?;
    settings::validate_pod_for_io(&current, &data_dir(state), pod_id)?;
    Ok(current)
}

pub fn validate_item_pods(
    current: &Settings,
    state: &AppState,
    items: &[StagedItem],
) -> Result<(), String> {
    let pod_ids: HashSet<u64> = items.iter().map(|item| item.pod_id as u64).collect();
    for pod_id in pod_ids {
        settings::validate_pod_for_io(current, &data_dir(state), pod_id)?;
    }
    Ok(())
}

pub fn item_path(item: &StagedItem, current: &Settings) -> Result<PathBuf, String> {
    let pod = current
        .pods
        .iter()
        .find(|pod| pod.id == item.pod_id as u64)
        .ok_or_else(|| format!("条目「{}」所属的匣已不存在", item.name))?;
    let root = crate::file_paths::resolve_path(Path::new(&pod.staging_folder))?;
    let raw = PathBuf::from(&item.staging_path);
    let name = raw
        .file_name()
        .ok_or_else(|| format!("条目「{}」的路径无效", item.name))?;
    let parent = raw
        .parent()
        .ok_or_else(|| format!("条目「{}」的路径无效", item.name))?;
    // 只解析父目录；删除前解析符号链接本身会误删它指向的目标。
    let safe_path = crate::file_paths::resolve_path(parent)?.join(name);
    if !crate::file_paths::path_is_within(&safe_path, &root)
        || crate::file_paths::paths_equal(&safe_path, &root)
    {
        return Err(format!("条目「{}」已不在当前匣的暂存目录内", item.name));
    }
    match fs::symlink_metadata(&raw) {
        Ok(metadata) if file_ops::is_reparse_or_symlink(&metadata) => {
            return Err(format!("条目「{}」是符号链接或目录重解析点", item.name));
        }
        Ok(_) => {
            let resolved = crate::file_paths::resolve_path(&raw)?;
            if !crate::file_paths::paths_equal(&resolved, &safe_path) {
                return Err(format!("条目「{}」的路径解析结果不一致", item.name));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("无法检查条目「{}」: {error}", item.name)),
    }
    Ok(safe_path)
}

pub(super) fn indexed_paths(state: &AppState, pod_id: u64) -> Result<HashSet<String>, String> {
    let connection = state.db.lock().unwrap();
    Ok(db::items_of_pod(&connection, pod_id as i64)?
        .into_iter()
        .map(|item| crate::file_paths::path_key(Path::new(&item.staging_path)))
        .collect())
}

pub(super) fn staging_directory(pod: &Pod) -> Result<PathBuf, String> {
    let directory = PathBuf::from(&pod.staging_folder);
    fs::create_dir_all(&directory).map_err(|error| format!("暂存文件夹不可用: {error}"))?;
    Ok(directory)
}
