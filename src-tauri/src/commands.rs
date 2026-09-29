//! Tauri 命令入口。文件和持久化逻辑位于独立模块，可脱离 WebView 测试。

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db::StagedItem;
use crate::export::ExportResult;
use crate::settings::{Hotkeys, Pod, PodPatch, Settings, SettingsPatch};
use crate::staging::StagePathsResult;
use crate::thumbnail::ThumbnailPayload;
use crate::{
    drag_out, export, handoff, logging, manager, operations, paths, pods, privacy, security,
    staging, thumbnail,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
#[path = "../contract/tests.rs"]
mod contract;

async fn blocking<T, F>(app: AppHandle, label: &'static str, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let permit = app.state::<crate::state::AppState>().tasks.enter()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        task()
    })
    .await
    .map_err(|error| format!("{label}后台任务异常终止：{error}"))?
}

#[derive(Serialize, schemars::JsonSchema)]
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

#[derive(Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataUsage {
    pub bytes: u64,
}

/// 统计数据目录（数据库、撤销区、日志）占用的磁盘字节数。
#[tauri::command]
pub async fn get_data_usage(app: AppHandle) -> Result<DataUsage, String> {
    blocking(app.clone(), "统计数据占用", move || {
        let state = app.state::<crate::state::AppState>();
        Ok(DataUsage {
            bytes: paths::directory_size(&state.data_dir),
        })
    })
    .await
}

/// 把数据目录迁移到新位置（复制数据库快照与撤销区，写入注册表覆盖项），
/// 重启应用后生效。迁移期间锁住文件操作，避免快照中途变化。
#[tauri::command]
pub async fn change_data_dir(app: AppHandle, new_path: String) -> Result<(), String> {
    blocking(app.clone(), "迁移数据目录", move || {
        let state = app.state::<crate::state::AppState>();
        let _file_operation = state.file_ops.lock().unwrap();
        let migrated = {
            let connection = state.db.lock().unwrap();
            paths::migrate_data_dir(&state.data_dir, &connection, &new_path)
        };
        if migrated.is_ok() {
            paths::set_override_data_dir(&new_path)?;
        }
        migrated
    })
    .await
}

/// 退出并重新启动应用（数据目录迁移后使新位置生效）。
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}

// Windows 上创建窗口必须使用异步 Tauri 命令，同步命令可能与 UI 消息循环死锁。
#[tauri::command]
pub async fn create_pod(
    app: AppHandle,
    config: PodPatch,
    reuse_existing: bool,
) -> Result<Pod, String> {
    pods::create(app, config.0, reuse_existing)
}

#[tauri::command]
pub async fn update_pod(app: AppHandle, pod_id: u64, patch: PodPatch) -> Result<Pod, String> {
    pods::update(app, pod_id, patch.0)
}

#[tauri::command]
pub async fn delete_pod(app: AppHandle, pod_id: u64, mode: String) -> Result<(), String> {
    pods::delete(app, pod_id, &mode)
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, patch: SettingsPatch) -> Result<Settings, String> {
    pods::save_settings(app, patch.0)
}

/// WebView 挂载后主动同步，弥补首次加载前未排队的事件。
#[tauri::command]
pub async fn get_panel_state(app: AppHandle, pod_id: u64) -> manager::PanelSnapshot {
    manager::panel_snapshot(&app, pod_id)
}

#[tauri::command]
pub async fn stage_paths(
    app: AppHandle,
    pod_id: u64,
    paths: Vec<String>,
    action: String,
) -> Result<StagePathsResult, String> {
    let history_app = app.clone();
    let history_paths = paths.clone();
    let history_action = action.clone();
    let result = blocking(app.clone(), "文件暂存", move || {
        staging::stage_paths(app, pod_id, paths, action)
    })
    .await;
    if let Err(error) = &result {
        let state = history_app.state::<crate::state::AppState>();
        let items = history_paths
            .iter()
            .map(|path| operations::OperationItemDraft {
                item_id: None,
                name: std::path::Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone()),
                source_path: Some(path.clone()),
                target_path: None,
                action: history_action.clone(),
                status: "failed".into(),
                error: Some(error.clone()),
                snapshot: None,
                compensation: None,
            })
            .collect();
        let _ = operations::record(
            &state.db.lock().unwrap(),
            operations::OperationDraft {
                kind: "stage".into(),
                pod_id: Some(pod_id as i64),
                summary: format!("暂存 {} 项失败", history_paths.len()),
                status: "failed".into(),
                undoable_until: None,
                metadata: serde_json::json!({
                    "retry": {
                        "podId": pod_id,
                        "paths": history_paths,
                        "action": history_action,
                    }
                }),
                items,
            },
        );
    }
    result
}

#[tauri::command]
pub async fn stage_text(
    app: AppHandle,
    pod_id: u64,
    content: String,
    title: Option<String>,
) -> Result<StagedItem, String> {
    blocking(app.clone(), "文字暂存", move || {
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
    blocking(app.clone(), "移出暂存项目", move || {
        staging::remove_items(app, ids, delete_files)
    })
    .await
}

#[tauri::command]
pub fn list_operations(
    app: AppHandle,
    hours: Option<u32>,
    limit: Option<u32>,
) -> Result<Vec<operations::OperationEntry>, String> {
    operations::list(&app, hours.unwrap_or(24), limit.unwrap_or(100))
}

#[tauri::command]
pub async fn undo_operation(
    app: AppHandle,
    operation_id: i64,
) -> Result<operations::UndoResult, String> {
    blocking(app.clone(), "撤销操作", move || {
        operations::undo(app, operation_id)
    })
    .await
}

#[tauri::command]
pub async fn retry_operation(
    app: AppHandle,
    operation_id: i64,
) -> Result<operations::RetryResult, String> {
    blocking(app.clone(), "重试失败项", move || {
        operations::retry(app, operation_id)
    })
    .await
}

#[tauri::command]
pub async fn preview_remove_items(
    app: AppHandle,
    ids: Vec<i64>,
    delete_files: bool,
) -> Result<operations::OperationPreview, String> {
    blocking(app.clone(), "预览移出操作", move || {
        operations::preview_remove(&app, &ids, delete_files)
    })
    .await
}

#[tauri::command]
pub async fn preview_export_items(
    app: AppHandle,
    ids: Vec<i64>,
    dest_dir: String,
    mode: String,
) -> Result<operations::OperationPreview, String> {
    blocking(app.clone(), "预览导出操作", move || {
        operations::preview_export(&app, &ids, &dest_dir, &mode)
    })
    .await
}

#[tauri::command]
pub async fn scan_privacy(
    app: AppHandle,
    ids: Vec<i64>,
) -> Result<privacy::PrivacyScanResult, String> {
    blocking(app.clone(), "本地隐私检查", move || {
        privacy::scan_items(&app, &ids)
    })
    .await
}

#[tauri::command]
pub async fn safe_export_items(
    app: AppHandle,
    ids: Vec<i64>,
    dest_dir: String,
) -> Result<privacy::SafeExportResult, String> {
    blocking(app.clone(), "生成隐私清理副本", move || {
        privacy::safe_export(app, ids, dest_dir)
    })
    .await
}

#[tauri::command]
pub async fn create_handoff(
    app: AppHandle,
    ids: Vec<i64>,
    dest_dir: String,
    title: String,
    note: String,
    clean_metadata: bool,
) -> Result<handoff::HandoffResult, String> {
    blocking(app.clone(), "生成可信交接包", move || {
        handoff::create(app, ids, dest_dir, title, note, clean_metadata)
    })
    .await
}

#[tauri::command]
pub async fn verify_handoff(
    app: AppHandle,
    directory: String,
) -> Result<handoff::VerifyResult, String> {
    blocking(app.clone(), "验证可信交接包", move || {
        handoff::verify(directory)
    })
    .await
}

#[tauri::command]
pub fn get_pod_security_status(
    app: AppHandle,
    pod_id: u64,
) -> Result<security::SecurityStatus, String> {
    security::status(&app, pod_id)
}

#[tauri::command]
pub async fn unlock_sensitive_pod(
    app: AppHandle,
    pod_id: u64,
) -> Result<security::SecurityStatus, String> {
    blocking(app.clone(), "Windows Hello 解锁", move || {
        security::unlock(&app, pod_id)
    })
    .await
}

#[tauri::command]
pub fn lock_sensitive_pod(app: AppHandle, pod_id: u64) {
    security::lock(&app, pod_id);
}

#[tauri::command]
pub fn lock_all_sensitive_pods(app: AppHandle) {
    security::lock_all(&app);
}

#[tauri::command]
pub async fn prepare_drag_cut(
    app: AppHandle,
    pod_id: u64,
    paths: Vec<String>,
) -> Result<String, String> {
    blocking(app.clone(), "准备剪切拖出", move || {
        drag_out::prepare(app, pod_id, paths)
    })
    .await
}

#[tauri::command]
pub async fn finalize_drag_cut(
    app: AppHandle,
    token: String,
) -> Result<drag_out::DragCutOutcome, String> {
    blocking(app.clone(), "剪切源清理", move || {
        drag_out::finalize(app, token)
    })
    .await
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
    blocking(app.clone(), "导出项目", move || {
        export::export_items(app, ids, dest_dir, mode, on_conflict)
    })
    .await
}

#[tauri::command]
pub async fn read_thumbnail(
    app: AppHandle,
    path: String,
) -> Result<Option<ThumbnailPayload>, String> {
    blocking(app.clone(), "读取缩略图", move || {
        thumbnail::read(app, path)
    })
    .await
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
    blocking(app.clone(), "打开文件", move || {
        staging::open_staged_item(&app, item_id)
    })
    .await
}

#[tauri::command]
pub async fn open_pod_folder(app: AppHandle, pod_id: u64) -> Result<(), String> {
    blocking(app.clone(), "打开文件夹", move || {
        staging::open_pod_folder(&app, pod_id)
    })
    .await
}

#[tauri::command]
pub async fn copy_staged_to_clipboard(app: AppHandle, item_ids: Vec<i64>) -> Result<(), String> {
    blocking(app.clone(), "复制到剪贴板", move || {
        staging::copy_staged_to_clipboard(&app, &item_ids)
    })
    .await
}

#[tauri::command]
pub async fn reveal_staged_items(app: AppHandle, item_ids: Vec<i64>) -> Result<(), String> {
    blocking(app.clone(), "打开所在位置", move || {
        staging::reveal_staged_items(&app, &item_ids)
    })
    .await
}

#[tauri::command]
pub fn write_clipboard_text(text: String) -> Result<(), String> {
    crate::clipboard::copy_text(&text)
}

#[tauri::command]
pub async fn read_clipboard_files(app: AppHandle) -> Result<Vec<String>, String> {
    blocking(app.clone(), "读取剪贴板文件", crate::clipboard::read_files).await
}

// ---- 右键菜单窗口 ----

#[tauri::command]
pub fn context_menu_ready(app: AppHandle) {
    crate::menu::mark_ready(&app);
}

#[tauri::command]
pub async fn open_context_menu(
    app: AppHandle,
    pod_id: u64,
    items: Vec<crate::menu::MenuItemSpec>,
) -> Result<(), String> {
    crate::menu::open(&app, pod_id, &items)
}

#[tauri::command]
pub async fn resize_context_menu(app: AppHandle, seq: u64, width: f64, height: f64) {
    crate::menu::resize_and_show(&app, seq, width, height);
}

#[tauri::command]
pub async fn context_menu_choice(
    app: AppHandle,
    seq: u64,
    pod_id: u64,
    action: crate::menu::MenuItemSpec,
) {
    crate::menu::choose(&app, seq, pod_id, &action);
}

#[tauri::command]
pub async fn hide_context_menu(app: AppHandle, seq: u64, pod_id: u64) {
    crate::menu::hide(&app, seq, pod_id);
}

/// 浮动面板在指针按下时主动收起当前菜单（不校验 seq，未打开时空操作）。
#[tauri::command]
pub fn dismiss_context_menu(app: AppHandle) {
    crate::menu::dismiss(&app);
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
