use std::fs;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::db::{self};
use crate::security;
use crate::settings::{self};
use crate::state::AppState;

use super::context::*;
/// 按 id 打开一个暂存条目：路径必须通过 item_path 校验（属于某匣的暂存目录、
/// 非 reparse 点），不能让 WebView 直接驱使系统打开任意路径。
pub fn open_staged_item(app: &AppHandle, item_id: i64) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let state = app.state::<AppState>();
    let (current, item) = {
        let connection = state.db.lock().unwrap();
        let item = db::items_by_ids(&connection, &[item_id])?
            .into_iter()
            .next()
            .ok_or_else(|| "条目不存在".to_string())?;
        (load_settings_from(&connection, &state)?, item)
    };
    security::require_items_unlocked(app, std::slice::from_ref(&item))?;
    settings::validate_pod_for_io(&current, &data_dir(&state), item.pod_id as u64)?;
    let path = item_path(&item, &current)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|error| format!("打开「{}」失败: {error}", item.name))
}

/// 打开匣的暂存文件夹（设置页入口）。文件夹经 resolve_path 解析并校验归属。
pub fn open_pod_folder(app: &AppHandle, pod_id: u64) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let state = app.state::<AppState>();
    security::require_unlocked(app, pod_id)?;
    let current = validated_settings(&state, pod_id)?;
    let pod = current
        .pods
        .iter()
        .find(|pod| pod.id == pod_id)
        .ok_or_else(|| "匣不存在".to_string())?;
    let path = crate::file_paths::resolve_path(Path::new(&pod.staging_folder))?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|error| format!("打开文件夹失败: {error}"))
}

/// 在资源管理器中定位暂存条目（右键菜单）。路径按条目 id 重新校验，
/// WebView 无法驱使系统定位任意路径。
pub fn reveal_staged_items(app: &AppHandle, item_ids: &[i64]) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let state = app.state::<AppState>();
    let (current, items) = {
        let connection = state.db.lock().unwrap();
        (
            load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, item_ids)?,
        )
    };
    security::require_items_unlocked(app, &items)?;
    let item = items.first().ok_or_else(|| "条目不存在".to_string())?;
    settings::validate_pod_for_io(&current, &data_dir(&state), item.pod_id as u64)?;
    let path = item_path(item, &current)?;
    app.opener()
        .reveal_item_in_dir(path.as_os_str())
        .map_err(|error| format!("打开「{}」所在位置失败: {error}", item.name))
}

/// 把暂存条目复制到系统剪贴板：纯文字条目复制文本内容，其余（含混合选择时
/// 的文字条目 txt 文件）按文件复制（CF_HDROP，可在资源管理器粘贴）。
/// 路径按条目 id 重新校验，与 open_staged_item 同一条安全边界。
pub fn copy_staged_to_clipboard(app: &AppHandle, item_ids: &[i64]) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (current, items) = {
        let connection = state.db.lock().unwrap();
        (
            load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, item_ids)?,
        )
    };
    if items.is_empty() {
        return Err("条目不存在".to_string());
    }
    security::require_items_unlocked(app, &items)?;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    for item in items {
        settings::validate_pod_for_io(&current, &data_dir(&state), item.pod_id as u64)?;
        let path = item_path(&item, &current)?;
        if item.kind == "text" && paths.is_empty() {
            let content = fs::read_to_string(&path)
                .map_err(|error| format!("读取「{}」失败: {error}", item.name))?;
            texts.push(content);
        } else {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        crate::clipboard::copy_text(&texts.join("\n\n"))
    } else {
        let refs: Vec<&Path> = paths.iter().map(|path| path.as_path()).collect();
        crate::clipboard::copy_files(&refs)
    }
}
