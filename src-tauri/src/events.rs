use tauri::{AppHandle, Emitter, Manager};

/// Rust 到前端的事件名，与 TypeScript 中的定义保持一致。
pub const ITEMS_CHANGED: &str = "floepod://items-changed";
pub const SETTINGS_CHANGED: &str = "floepod://settings-changed";
pub const PODS_CHANGED: &str = "floepod://pods-changed";
pub const PANEL_MODE: &str = "floepod://panel-mode";
pub const PANEL_SHOWN: &str = "floepod://panel-shown";
pub const PANEL_PINNED: &str = "floepod://panel-pinned";
/// 完整浮动面板运行态快照；用于 WebView 首次挂载后的主动同步与事件丢失恢复。
pub const PANEL_STATE: &str = "floepod://panel-state";
pub const PANEL_HIDDEN: &str = "floepod://panel-hidden";
pub const COLLECT_CLIPBOARD: &str = "floepod://collect-clipboard";
pub const REQUEST_FILE_PICKER: &str = "floepod://request-file-picker";
pub const POD_LOCK_CHANGED: &str = "floepod://pod-lock-changed";
/// 隐匿模式状态变化：边缘浮动条应淡化隐去（true）或淡入显示（false）。
pub const BAR_STEALTH: &str = "floepod://bar-stealth";

pub fn pod_bar_label(pod_id: u64) -> String {
    format!("pod_{pod_id}")
}

pub fn pod_panel_label(pod_id: u64) -> String {
    format!("pod_{pod_id}_panel")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PodWindow {
    Bar(u64),
    Panel(u64),
}

pub fn pod_window(label: &str) -> Option<PodWindow> {
    let value = label.strip_prefix("pod_")?;
    if let Some(id) = value.strip_suffix("_panel") {
        id.parse().ok().filter(|id| *id > 0).map(PodWindow::Panel)
    } else {
        value.parse().ok().filter(|id| *id > 0).map(PodWindow::Bar)
    }
}

/// 条目变更只发送给对应匣的两个 WebView；停用匣或切换设置时窗口不存在属于正常情况。
pub fn emit_items_changed(app: &AppHandle, pod_id: u64) {
    let payload = serde_json::json!({ "podId": pod_id });
    for label in [pod_bar_label(pod_id), pod_panel_label(pod_id)] {
        if app.get_webview_window(&label).is_some() {
            let _ = app.emit_to(label, ITEMS_CHANGED, payload.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_window_labels_round_trip_without_cross_talk() {
        for pod_id in [1, 2, u32::MAX as u64 + 1] {
            let bar = pod_bar_label(pod_id);
            let panel = pod_panel_label(pod_id);
            assert_eq!(pod_window(&bar), Some(PodWindow::Bar(pod_id)));
            assert_eq!(pod_window(&panel), Some(PodWindow::Panel(pod_id)));
            assert_ne!(bar, pod_bar_label(pod_id + 1));
            assert_ne!(panel, pod_panel_label(pod_id + 1));
        }
    }

    #[test]
    fn rejects_settings_malformed_and_zero_pod_labels() {
        for label in [
            "settings",
            "pod_0",
            "pod_0_panel",
            "pod_-1",
            "pod_1_extra",
            "pod_1_panel_extra",
        ] {
            assert_eq!(pod_window(label), None, "{label}");
        }
    }
}
