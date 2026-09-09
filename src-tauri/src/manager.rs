//! Window orchestration: apply a validated settings snapshot across independent window domains.
use crate::settings::{Pod, Settings};
use crate::state::AppState;
use crate::{events, win};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};
mod auto_block;
mod geometry;
mod material;
mod panel;
mod screens;
mod watchdog;
mod windows;

pub use auto_block::spawn_auto_block_watcher;
use material::apply_panel_material_if_changed;
pub(crate) use material::{apply_window_material, refresh_pod_bar_chrome, refresh_window_material};
use panel::emit_panel_snapshot;
pub use panel::{
    hide_panel, hold_pending_drop, panel_snapshot, report_presence, set_all_bars, set_dragging_out,
    set_panel_mode, set_panel_pinned, set_panel_size, show_panel, toggle_panel, PanelSnapshot,
};
pub use screens::{list_monitors, MonitorInfo};
pub use watchdog::spawn_watchdog;
pub use windows::{move_pod_bar, open_settings, place_pod_bar, pod_bar, pod_panel, set_pod_accept};
use windows::{place_panel, sync_pods_with_settings};

/// 严格版设置读取：加载 + 完整校验，供 watcher 等必须拿到可信配置的路径使用。
pub fn load_validated_settings(app: &AppHandle) -> Result<Settings, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let settings = crate::settings::load(
        &conn,
        &state.data_dir.to_string_lossy(),
        env!("CARGO_PKG_VERSION"),
    )?;
    crate::settings::validate(&settings, &state.data_dir.to_string_lossy())?;
    Ok(settings)
}

pub fn current_settings(app: &AppHandle) -> Settings {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    match crate::settings::load(
        &conn,
        &state.data_dir.to_string_lossy(),
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(settings) => settings,
        Err(error) => {
            crate::logging::write(&format!("[settings] 读取当前设置失败: {error}"));
            Settings::default()
        }
    }
}

fn pod_of(app: &AppHandle, id: u64) -> Option<Pod> {
    current_settings(app)
        .pods
        .into_iter()
        .find(|pod| pod.id == id && pod.enabled)
}

/// 把与运行态相关的配置同步进 PodRuntime（自动隐藏 / 隐匿模式设置）。
/// 浮动面板材质不在这里处理：必须对隐藏的浮动面板也落地，见 apply_panel_material_if_changed。
fn sync_runtime_config(app: &AppHandle, pod: &Pod) {
    let state = app.state::<AppState>();
    let mut guard = state.pods.lock().unwrap();
    let runtime = guard.entry(pod.id).or_default();
    runtime.auto_hide_enabled = pod.auto_hide;
    runtime.auto_hide_delay_ms = pod.auto_hide_delay_ms;
    runtime.stealth_enabled = pod.stealth;
    runtime.stealth_delay_ms = pod.stealth_delay_ms;
}

/// 同步系统开机自启动状态。调用方负责在持久化设置时把此副作用纳入事务回滚。
pub fn sync_autostart(_app: &AppHandle, enabled: bool) -> Result<(), String> {
    crate::autostart::sync(enabled).map_err(|e| {
        format!(
            "{}开机自启动失败: {e}",
            if enabled { "启用" } else { "禁用" }
        )
    })
}

/// 设置落地：同步匣窗口、材质、监听、托盘全量应用。
/// 自启动属于可失败的系统副作用，由保存设置和启动流程显式调用 `sync_autostart`。
pub fn apply_settings(app: &AppHandle, s: &Settings) {
    if let Err(error) = crate::shell_integration::sync(s.accessibility.send_to_menu) {
        crate::logging::write(&format!("[shell] 同步资源管理器菜单失败: {error}"));
    }
    // 自动屏蔽配置驻留内存：轮询线程每几百毫秒读取一次，不能每次都查库。
    {
        let state = app.state::<AppState>();
        state
            .auto_block_enabled
            .store(s.auto_block.enabled, Ordering::Relaxed);
        *state.auto_block_apps.lock().unwrap() = s.auto_block.apps.clone();
        state
            .accessibility_reduce_transparency
            .store(s.accessibility.reduce_transparency, Ordering::Relaxed);
    }

    // 窗口创建/销毁与所有浮动面板显隐串行。对已固定、可见的浮动面板立即应用新的
    // monitor/edge/offset/panelWidth/material，而不是等到关闭重开。
    {
        let state = app.state::<AppState>();
        let _operation = state.panel_ops.lock().unwrap();
        sync_pods_with_settings(app, s);

        for pod in s.pods.iter().filter(|p| p.enabled) {
            sync_runtime_config(app, pod);
            // 浮动面板材质无论可见与否都要落地，否则未固定的浮动面板改材质永远不生效。
            apply_panel_material_if_changed(app, pod);
            place_pod_bar(app, pod, false);
            if let Some(panel) = pod_panel(app, pod.id) {
                let _ = panel.set_title(&format!("{} 浮动面板", pod.name));
            }
            let panel_visible = state
                .pods
                .lock()
                .unwrap()
                .get(&pod.id)
                .map(|runtime| runtime.panel_visible)
                .unwrap_or(false);
            if panel_visible {
                place_panel(app, pod);
                emit_panel_snapshot(app, pod.id);
            }
        }
    }

    // 暂存文件夹监听（每个匣一个）
    crate::watcher::restart_all(app);

    // 配置完成（OOBE 结束）后亮相
    if s.first_run_done
        && !s.pods.is_empty()
        && app.state::<AppState>().bars_visible.load(Ordering::Relaxed)
    {
        for pod in s.pods.iter().filter(|p| p.enabled) {
            if let Some(bar) = pod_bar(app, pod.id) {
                if let Ok(hwnd) = bar.hwnd() {
                    win::show_bar_no_activate(hwnd.0 as isize);
                }
            }
        }
    }

    crate::tray::refresh_menu(app);
    let _ = events::SETTINGS_CHANGED.emit(app, s.clone());
}

#[cfg(test)]
mod tests;
