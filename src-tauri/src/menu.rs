//! 应用级右键菜单窗口：全局唯一的透明置顶窗口，由所有匣面板复用。
//!
//! 流程：面板 invoke `open_context_menu`（携带菜单项）→ 本模块发出定向事件
//! → 菜单窗口渲染并回传内容尺寸 → `resize_and_show` 把窗口定位到光标旁并
//! 显示。动作选择通过 `context_menu_choice` 回传给来源面板执行，菜单窗口
//! 自身不直接触碰条目数据。seq 序号消解新旧菜单竞态：旧菜单的 blur / 关闭
//! 请求不会影响刚打开的新菜单。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

use crate::events;
use crate::win;

pub const LABEL: &str = "context_menu";
pub const MENU_SHOW: &str = "floepod://context-menu-show";
pub const MENU_CHOICE: &str = "floepod://context-menu-choice";
pub const MENU_CLOSED: &str = "floepod://context-menu-closed";

static MENU_SEQ: AtomicU64 = AtomicU64::new(0);
/// 菜单窗口 WebView 已挂载并上报就绪；未就绪时 open 直接报错，面板走内嵌降级。
static MENU_READY: AtomicBool = AtomicBool::new(false);
/// 当前菜单归属的匣 id；0 表示从未打开。
static MENU_POD: AtomicU64 = AtomicU64::new(0);
/// 是否存在未关闭的菜单（open 置位、hide 复位）。
static MENU_OPEN: AtomicBool = AtomicBool::new(false);

/// 菜单项描述：面板组装、菜单窗口渲染、选择后原样回传面板执行。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MenuItemSpec {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub separator: bool,
    #[serde(default)]
    pub danger: bool,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub item_ids: Vec<i64>,
    #[serde(default)]
    pub text: String,
}

pub fn mark_ready(app: &AppHandle) {
    // 静态声明的菜单窗口圆角由 CSS 绘制，禁用系统圆角避免裁切。
    if let Some(window) = app.get_webview_window(LABEL) {
        if let Ok(hwnd) = window.hwnd() {
            win::disable_rounding(hwnd.0 as isize);
        }
    }
    MENU_READY.store(true, Ordering::Relaxed);
}

pub fn open(app: &AppHandle, pod_id: u64, items: &[MenuItemSpec]) -> Result<(), String> {
    if !MENU_READY.load(Ordering::Relaxed) {
        return Err("CONTEXT_MENU_NOT_READY".into());
    }
    let seq = MENU_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let previous_pod = MENU_POD.swap(pod_id, Ordering::Relaxed);
    // 旧菜单的 blur 关闭请求会因 seq 校验被忽略（不能误杀新菜单），
    // 这里对被取代的旧归属匣补发 CLOSED，保证其保活状态总能被解除。
    if previous_pod != 0 && previous_pod != pod_id && MENU_OPEN.load(Ordering::Relaxed) {
        let _ = app.emit_to(
            events::pod_panel_label(previous_pod),
            MENU_CLOSED,
            serde_json::json!({ "podId": previous_pod }),
        );
    }
    MENU_OPEN.store(true, Ordering::Relaxed);
    app.emit_to(
        LABEL,
        MENU_SHOW,
        serde_json::json!({ "seq": seq, "podId": pod_id, "items": items }),
    )
    .map_err(|error| format!("菜单窗口事件发送失败: {error}"))
}

pub fn resize_and_show(app: &AppHandle, seq: u64, width: f64, height: f64) {
    if seq != MENU_SEQ.load(Ordering::Relaxed) {
        return;
    }
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let size = PhysicalSize::new(
        (width.max(1.0) * scale).round() as u32,
        (height.max(1.0) * scale).round() as u32,
    );
    let position = clamp_to_monitor(app, cursor_position(), size);
    let _ = window.set_size(size);
    let _ = window.set_position(position);
    // 显示必须与 hide 的原生 ShowWindow 路径对称：Tauri 的 show() 会同步
    // WebView2 的可见性状态，与 SW_HIDE 混用后透明窗口内容停留在未恢复的
    // 合成状态，菜单第二次起就再也显示不出来。
    match window.hwnd() {
        Ok(hwnd) => win::show_window(hwnd.0 as isize),
        Err(_) => {
            let _ = window.show();
        }
    }
    // 菜单抢焦点后才能用 blur 检测「点击菜单外部」并自动关闭。
    let _ = window.set_focus();
}

pub fn choose(app: &AppHandle, seq: u64, pod_id: u64, action: &MenuItemSpec) {
    if seq != MENU_SEQ.load(Ordering::Relaxed) {
        return;
    }
    let _ = app.emit_to(
        events::pod_panel_label(pod_id),
        MENU_CHOICE,
        serde_json::json!({ "podId": pod_id, "action": action }),
    );
}

pub fn hide(app: &AppHandle, seq: u64, pod_id: u64) {
    if seq != MENU_SEQ.load(Ordering::Relaxed) {
        // 旧菜单的失焦关闭请求到达时新菜单已打开，不能误杀；
        // 其归属面板由 open() 的补发 CLOSED 恢复。
        return;
    }
    MENU_OPEN.store(false, Ordering::Relaxed);
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    if let Ok(hwnd) = window.hwnd() {
        // 必须走 ShowWindow 隐藏：Tauri 的 hide() 对 WebView2 的处理
        // 会让透明窗口残留（与面板隐藏同一条约束）。
        win::hide_window(hwnd.0 as isize);
    }
    let _ = app.emit_to(
        events::pod_panel_label(pod_id),
        MENU_CLOSED,
        serde_json::json!({ "podId": pod_id }),
    );
}

fn cursor_position() -> (i32, i32) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut point = POINT { x: 0, y: 0 };
    unsafe {
        if GetCursorPos(&mut point) != 0 {
            return (point.x, point.y);
        }
    }
    (0, 0)
}

/// 菜单以光标为左上角，clamp 在光标所在显示器内；找不到就用主显示器兜底。
fn clamp_to_monitor(
    app: &AppHandle,
    cursor: (i32, i32),
    size: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    let mut bounds = None;
    for monitor in app.available_monitors().unwrap_or_default() {
        let position = monitor.position();
        let scale = monitor.size();
        if cursor.0 >= position.x
            && cursor.0 < position.x + scale.width as i32
            && cursor.1 >= position.y
            && cursor.1 < position.y + scale.height as i32
        {
            bounds = Some((
                position.x,
                position.y,
                scale.width as i32,
                scale.height as i32,
            ));
            break;
        }
    }
    if bounds.is_none() {
        if let Ok(Some(monitor)) = app.primary_monitor() {
            let position = monitor.position();
            let scale = monitor.size();
            bounds = Some((
                position.x,
                position.y,
                scale.width as i32,
                scale.height as i32,
            ));
        }
    }
    let (x, y, width, height) = bounds.unwrap_or((0, 0, 1920, 1080));
    let window_w = size.width as i32;
    let window_h = size.height as i32;
    PhysicalPosition::new(
        cursor.0.clamp(x, (x + width - window_w).max(x)),
        cursor.1.clamp(y, (y + height - window_h).max(y)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str) -> MenuItemSpec {
        MenuItemSpec {
            id: id.into(),
            label: id.into(),
            separator: false,
            danger: false,
            disabled: false,
            item_ids: vec![1],
            text: String::new(),
        }
    }

    #[test]
    fn menu_item_spec_round_trips_camel_case() {
        let mut spec = item("copy");
        spec.item_ids = vec![3, 9];
        let value = serde_json::to_value(&spec).unwrap();
        assert_eq!(value["itemIds"], serde_json::json!([3, 9]));
        let back: MenuItemSpec = serde_json::from_value(value).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn menu_item_spec_defaults_tolerate_missing_fields() {
        let spec: MenuItemSpec =
            serde_json::from_value(serde_json::json!({ "id": "sep", "separator": true })).unwrap();
        assert!(spec.separator);
        assert!(spec.item_ids.is_empty());
        assert!(!spec.danger);
    }
}
