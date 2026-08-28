//! Tauri 命令入口。文件和持久化逻辑位于独立模块，可脱离 WebView 测试。

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db::StagedItem;
use crate::export::ExportResult;
use crate::settings::{Hotkeys, Pod, Settings};
use crate::staging::StagePathsResult;
use crate::thumbnail::ThumbnailPayload;
use crate::{drag_out, export, logging, manager, pods, staging, thumbnail};

const VERSION: &str = env!("CARGO_PKG_VERSION");

async fn blocking<T, F>(label: &'static str, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| format!("{label}后台任务异常终止：{error}"))?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    settings: Settings,
    monitors: Vec<manager::MonitorInfo>,
    version: String,
}

#[tauri::command]
pub fn get_bootstrap(app: AppHandle) -> Result<Bootstrap, String> {
    let state = app.state::<crate::state::AppState>();
    let settings = staging::load_settings(&state)?;
    Ok(Bootstrap {
        settings,
        monitors: manager::list_monitors(&app),
        version: VERSION.to_string(),
    })
}

#[tauri::command]
pub fn get_modifier_state() -> crate::win::ModifierState {
    crate::win::modifier_state()
}

#[tauri::command]
pub fn get_hotkey_defaults() -> Hotkeys {
    Hotkeys::with_defaults()
}

// Windows 上创建窗口必须使用异步 Tauri 命令，同步命令可能与 UI 消息循环死锁。
#[tauri::command]
pub async fn create_pod(
    app: AppHandle,
    config: serde_json::Value,
    reuse_existing: bool,
) -> Result<Pod, String> {
    pods::create(app, config, reuse_existing)
}

#[tauri::command]
pub async fn update_pod(
    app: AppHandle,
    pod_id: u64,
    patch: serde_json::Value,
) -> Result<Pod, String> {
    pods::update(app, pod_id, patch)
}

#[tauri::command]
pub async fn delete_pod(app: AppHandle, pod_id: u64, recycle_files: bool) -> Result<(), String> {
    pods::delete(app, pod_id, recycle_files)
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, patch: serde_json::Value) -> Result<Settings, String> {
    pods::save_settings(app, patch)
}

#[tauri::command]
pub async fn stage_paths(
    app: AppHandle,
    pod_id: u64,
    paths: Vec<String>,
    action: String,
) -> Result<StagePathsResult, String> {
    blocking("文件暂存", move || {
        staging::stage_paths(app, pod_id, paths, action)
    })
    .await
}

#[tauri::command]
pub async fn stage_text(
    app: AppHandle,
    pod_id: u64,
    content: String,
    title: Option<String>,
) -> Result<StagedItem, String> {
    blocking("文字暂存", move || {
        staging::stage_text(app, pod_id, content, title)
    })
    .await
}

#[tauri::command]
pub fn list_pod_items(app: AppHandle, pod_id: u64) -> Result<Vec<StagedItem>, String> {
    staging::list_pod_items(&app, pod_id)
}

#[tauri::command]
pub async fn remove_items(app: AppHandle, ids: Vec<i64>, delete_files: bool) -> Result<(), String> {
    blocking("移出暂存项目", move || {
        staging::remove_items(app, ids, delete_files)
    })
    .await
}

#[tauri::command]
pub async fn prepare_drag_cut(
    app: AppHandle,
    pod_id: u64,
    paths: Vec<String>,
) -> Result<String, String> {
    blocking("准备剪切拖出", move || {
        drag_out::prepare(app, pod_id, paths)
    })
    .await
}

#[tauri::command]
pub async fn finalize_drag_cut(app: AppHandle, token: String) -> Result<(), String> {
    blocking("剪切源清理", move || drag_out::finalize(app, token)).await
}

#[tauri::command]
pub fn cancel_drag_cut(app: AppHandle, token: String) {
    drag_out::cancel(&app, &token);
}

#[tauri::command]
pub async fn export_items(
    app: AppHandle,
    ids: Vec<i64>,
    dest_dir: String,
    mode: String,
    on_conflict: String,
) -> Result<ExportResult, String> {
    blocking("导出项目", move || {
        export::export_items(app, ids, dest_dir, mode, on_conflict)
    })
    .await
}

#[tauri::command]
pub async fn read_thumbnail(
    app: AppHandle,
    path: String,
) -> Result<Option<ThumbnailPayload>, String> {
    blocking("读取缩略图", move || thumbnail::read(app, path)).await
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
pub async fn set_panel_size(app: AppHandle, pod_id: u64, height: u32) {
    manager::set_panel_size(&app, pod_id, height);
}

#[tauri::command]
pub async fn move_pod_bar(app: AppHandle, pod_id: u64, offset: f64) -> Result<(), String> {
    manager::move_pod_bar(&app, pod_id, offset);
    Ok(())
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    manager::open_settings(&app);
}

#[tauri::command]
pub async fn open_staged_item(app: AppHandle, item_id: i64) -> Result<(), String> {
    blocking("打开文件", move || {
        staging::open_staged_item(&app, item_id)
    })
    .await
}

#[tauri::command]
pub async fn open_pod_folder(app: AppHandle, pod_id: u64) -> Result<(), String> {
    blocking("打开文件夹", move || {
        staging::open_pod_folder(&app, pod_id)
    })
    .await
}

#[tauri::command]
pub fn log_frontend(msg: String) {
    logging::write(&format!("[frontend] {msg}"));
}

#[tauri::command]
pub fn app_log(msg: String) {
    logging::write(&format!("[ui] {msg}"));
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}
