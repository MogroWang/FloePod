//! 全局快捷键注册与分发。

use std::collections::HashMap;
use std::str::FromStr;

use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::events;
use crate::manager;
use crate::settings::{Hotkeys, Settings};

fn validate(hotkeys: &Hotkeys) -> Result<(), String> {
    let entries = [
        ("显示 / 隐藏浮匣", hotkeys.toggle_bar.as_str()),
        ("收集剪贴板", hotkeys.collect_clipboard.as_str()),
        ("打开面板", hotkeys.open_panel.as_str()),
    ];
    let mut seen: HashMap<u32, &str> = HashMap::new();
    for (label, combo) in entries {
        if combo.is_empty() {
            continue;
        }
        if combo.trim() != combo {
            return Err(format!("快捷键「{label}」前后不能包含空格"));
        }
        let shortcut =
            Shortcut::from_str(combo).map_err(|e| format!("快捷键「{label}」格式无效（{e}）"))?;
        if let Some(previous) = seen.insert(shortcut.id(), label) {
            return Err(format!("快捷键「{previous}」与「{label}」不能相同"));
        }
    }
    Ok(())
}

pub fn register(app: &AppHandle, s: &Settings) -> Result<(), String> {
    // 在注销当前快捷键之前先完成纯解析与重复校验。这样格式错误不会让旧快捷键失效。
    validate(&s.hotkeys)?;
    let gs = app.global_shortcut();
    gs.unregister_all()
        .map_err(|e| format!("注销旧快捷键失败（{e}）"))?;

    let reg = |combo: &str, action: fn(&AppHandle)| -> Result<(), String> {
        if combo.is_empty() {
            return Ok(());
        }
        gs.on_shortcut(combo, move |app, _shortcut, e| {
            if e.state() == ShortcutState::Pressed {
                action(app);
            }
        })
        .map_err(|err| format!("快捷键「{combo}」注册失败，可能与其他软件冲突（{err}）"))
    };

    let registration = (|| {
        reg(&s.hotkeys.toggle_bar, on_toggle_bars)?;
        reg(&s.hotkeys.collect_clipboard, |app| {
            if let Some(id) = collect_into_first_pod(app) {
                let _ = app.emit_to(
                    events::pod_bar_label(id),
                    events::COLLECT_CLIPBOARD,
                    serde_json::json!({ "podId": id }),
                );
            }
        })?;
        reg(&s.hotkeys.open_panel, on_open_panel)
    })();

    if let Err(error) = registration {
        // Do not leave an arbitrary prefix of the new shortcut set active.  The caller can
        // now safely restore the previous complete set (or startup can continue with none).
        return match gs.unregister_all() {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!("{error}；清理已部分注册的快捷键失败（{cleanup}）")),
        };
    }
    Ok(())
}

fn on_toggle_bars(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::tray::toggle_bars(&app);
    });
}

/// 打开第一个可用匣的面板。
fn on_open_panel(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let id = manager::current_settings(&app)
            .pods
            .into_iter()
            .find(|p| p.enabled)
            .map(|p| p.id);
        if let Some(id) = id {
            manager::toggle_panel(&app, id);
        }
    });
}

/// 收集剪贴板：把文字暂存到第一个可用匣。
pub fn collect_into_first_pod(app: &AppHandle) -> Option<u64> {
    manager::current_settings(app)
        .pods
        .into_iter()
        .find(|p| p.enabled)
        .map(|p| p.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert!(validate(&Hotkeys::with_defaults()).is_ok());
    }

    #[test]
    fn rejects_duplicate_shortcuts_even_with_different_spelling() {
        let hotkeys = Hotkeys {
            toggle_bar: "Alt+Shift+F".into(),
            collect_clipboard: "shift+alt+KeyF".into(),
            open_panel: String::new(),
        };
        assert!(validate(&hotkeys).is_err());
    }

    #[test]
    fn rejects_invalid_shortcut_before_registration() {
        let hotkeys = Hotkeys {
            toggle_bar: "Alt+NotARealKey".into(),
            collect_clipboard: String::new(),
            open_panel: String::new(),
        };
        assert!(validate(&hotkeys).is_err());
    }
}
