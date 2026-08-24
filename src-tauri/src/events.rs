/** Rust -> 前端事件名（与 src/lib/events.ts 保持一致） */
pub const ITEMS_CHANGED: &str = "floepod://items-changed";
pub const SETTINGS_CHANGED: &str = "floepod://settings-changed";
pub const PODS_CHANGED: &str = "floepod://pods-changed";
pub const PANEL_MODE: &str = "floepod://panel-mode";
pub const PANEL_SHOWN: &str = "floepod://panel-shown";
pub const PANEL_PINNED: &str = "floepod://panel-pinned";
/// 完整面板运行态快照；用于 WebView 首次挂载后的主动同步与事件丢失恢复。
pub const PANEL_STATE: &str = "floepod://panel-state";
pub const PANEL_HIDDEN: &str = "floepod://panel-hidden";
pub const COLLECT_CLIPBOARD: &str = "floepod://collect-clipboard";
