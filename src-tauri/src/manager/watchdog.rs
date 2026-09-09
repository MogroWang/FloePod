//! Owned periodic presence/stealth work; no detached timer per hide.
use super::panel::{finish_delayed_hides, transition_to_hidden_locked};
use crate::state::AppState;
use crate::{events, win};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
const STEALTH_PROXIMITY: f64 = 48.0;
/// 光标是否落在矩形外扩 proximity（按缩放率换算为物理像素）的范围内。
pub(super) fn cursor_near(rect: (i32, i32, i32, i32), scale: f64) -> bool {
    let Some((cx, cy)) = win::cursor_pos() else {
        return false;
    };
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let pad = (STEALTH_PROXIMITY * scale).round() as i32;
    let (x, y, w, h) = rect;
    cx >= x - pad && cx <= x + w + pad && cy >= y - pad && cy <= y + h + pad
}

/// 更新边缘浮动条的隐匿状态并按需通知前端。
pub(super) fn set_bar_stealth(app: &AppHandle, id: u64, hidden: bool) {
    {
        let state = app.state::<AppState>();
        state
            .pods
            .lock()
            .unwrap()
            .entry(id)
            .or_default()
            .bar_stealth_hidden = hidden;
    }
    let label = events::pod_bar_label(id);
    if app.get_webview_window(&label).is_some() {
        let _ = events::BAR_STEALTH.emit_to(app, label, crate::events::StealthChanged { hidden });
    }
}

/// 隐匿模式判定：边缘浮动条仅在「开启隐匿、浮动面板未打开、无交互超过延迟、且
/// 指针不在附近」时淡化隐去；任一条件不满足则恢复显示。状态变化时向
/// 边缘浮动条窗口发事件，由前端做透明度过渡，窗口本身保持可交互（拖文件到
/// 原位置仍能暂存）。由看门狗每个 tick 调用，全部读取内存运行态。
pub(super) fn update_bar_stealth(app: &AppHandle, id: u64, now: Instant) {
    let state = app.state::<AppState>();
    let (enabled, delay_ms, hidden, rect, scale, panel_visible, last_change) = {
        let guard = state.pods.lock().unwrap();
        match guard.get(&id) {
            Some(runtime) => (
                runtime.stealth_enabled,
                runtime.stealth_delay_ms,
                runtime.bar_stealth_hidden,
                runtime.bar_rect,
                runtime.bar_scale,
                runtime.panel_visible,
                runtime.last_change,
            ),
            None => return,
        }
    };
    // last_change 缺失（从未交互）视为已超时：隐匿匣在启动 / 恢复显示后
    // 若指针不在附近，到期即淡出。
    let should_hide = enabled
        && !panel_visible
        && last_change
            .map(|changed| now.saturating_duration_since(changed) > Duration::from_millis(delay_ms))
            .unwrap_or(true)
        && !rect.map(|rect| cursor_near(rect, scale)).unwrap_or(false);
    if should_hide != hidden {
        set_bar_stealth(app, id, should_hide);
    }
}

/// 看门狗：逐个匣检查--浮动面板未固定、未在拖出、列表模式且指针离开超过宽限期 -> 淡出隐藏。
/// 单一活动浮动面板由 show_panel / report_presence 主动维持；这里只负责指针离开后的兜底隐藏。
/// 关闭了「匣浮动面板自动收起」的匣不参与自动隐藏，浮动面板保持到用户手动关闭。
/// 隐匿模式（边缘浮动条淡出 / 淡入）也在同一循环里判定。
pub fn spawn_watchdog(app: AppHandle) {
    let owner = app.clone();
    if let Err(error) = owner.state::<AppState>().watchdog_task.start(
        "panel-watchdog",
        Duration::from_millis(100),
        move |stop| {
            let state = app.state::<AppState>();
            let _operation = state.panel_ops.lock().unwrap();
            let now = Instant::now();
            finish_delayed_hides(&app, now);
            if !state.bars_visible.load(Ordering::Relaxed) {
                return Ok(());
            }
            let ids: Vec<u64> = state.pods.lock().unwrap().keys().copied().collect();
            for id in ids {
                if stop.is_stopped() {
                    break;
                }
                transition_to_hidden_locked(&app, id, |runtime| {
                    runtime.auto_hide_enabled
                        && runtime
                            .can_auto_hide(now, Duration::from_millis(runtime.auto_hide_delay_ms))
                });
                update_bar_stealth(&app, id, now);
            }
            Ok(())
        },
    ) {
        crate::logging::write(&format!("[watchdog] {error}"));
    }
}
