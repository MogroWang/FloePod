use serde::Serialize;
use std::marker::PhantomData;
use tauri::{AppHandle, Emitter, Manager};

/// A named event owns its payload type. Native emission and generated TS use this same definition.
pub struct Event<T> {
    name: &'static str,
    marker: PhantomData<fn() -> T>,
}
impl<T: Serialize + Clone> Event<T> {
    const fn new(name: &'static str) -> Self {
        Self {
            name,
            marker: PhantomData,
        }
    }
    pub fn emit(&self, app: &AppHandle, payload: T) -> tauri::Result<()> {
        app.emit(self.name, payload)
    }
    pub fn emit_to(
        &self,
        app: &AppHandle,
        target: impl Into<tauri::EventTarget>,
        payload: T,
    ) -> tauri::Result<()> {
        app.emit_to(target, self.name, payload)
    }
}

#[derive(Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PodEvent {
    pub pod_id: u64,
}
#[derive(Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PodLockChanged {
    pub pod_id: u64,
    pub locked: bool,
}
#[derive(Clone, Serialize, schemars::JsonSchema)]
pub struct ModeChanged {
    #[schemars(extend("enum" = ["list","ask","conflict"]))]
    pub mode: String,
    pub paths: Vec<String>,
}
#[derive(Clone, Serialize, schemars::JsonSchema)]
pub struct PinnedChanged {
    pub pinned: bool,
}
#[derive(Clone, Serialize, schemars::JsonSchema)]
pub struct StealthChanged {
    pub hidden: bool,
}
#[derive(Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MenuShow {
    pub seq: u64,
    pub pod_id: u64,
    pub items: Vec<crate::menu::MenuItemSpec>,
    #[schemars(extend("enum" = ["plain","acrylic"]))]
    pub material: String,
}
#[derive(Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MenuChoice {
    pub pod_id: u64,
    pub action: crate::menu::MenuItemSpec,
}

macro_rules! declare_events {
    ($($name:ident: $payload:ty = $wire:literal),+ $(,)?) => {
        $(pub const $name: Event<$payload> = Event::new($wire);)+
        #[cfg(test)]
        pub fn schemas() -> serde_json::Value {
            let mut schemas = serde_json::Map::new();
            $(schemas.insert($wire.into(), serde_json::to_value(schemars::generate::SchemaSettings::draft2020_12().for_serialize().into_generator().into_root_schema_for::<$payload>()).unwrap());)+
            serde_json::Value::Object(schemas)
        }
    }
}

declare_events! {
    ITEMS_CHANGED: PodEvent = "floepod://items-changed",
    SETTINGS_CHANGED: crate::settings::Settings = "floepod://settings-changed",
    PODS_CHANGED: () = "floepod://pods-changed",
    PANEL_MODE: ModeChanged = "floepod://panel-mode",
    PANEL_SHOWN: () = "floepod://panel-shown",
    PANEL_PINNED: PinnedChanged = "floepod://panel-pinned",
    PANEL_STATE: crate::manager::PanelSnapshot = "floepod://panel-state",
    PANEL_HIDDEN: () = "floepod://panel-hidden",
    COLLECT_CLIPBOARD: PodEvent = "floepod://collect-clipboard",
    REQUEST_FILE_PICKER: PodEvent = "floepod://request-file-picker",
    POD_LOCK_CHANGED: PodLockChanged = "floepod://pod-lock-changed",
    BAR_STEALTH: StealthChanged = "floepod://bar-stealth",
    MENU_SHOW: MenuShow = "floepod://context-menu-show",
    MENU_CHOICE: MenuChoice = "floepod://context-menu-choice",
    MENU_CLOSED: PodEvent = "floepod://context-menu-closed",
}

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
    let payload = PodEvent { pod_id };
    for label in [pod_bar_label(pod_id), pod_panel_label(pod_id)] {
        if app.get_webview_window(&label).is_some() {
            let _ = ITEMS_CHANGED.emit_to(app, label, payload.clone());
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
