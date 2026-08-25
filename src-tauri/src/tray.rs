//! 托盘：常驻入口与各「匣」的快捷入口。

use tauri::menu::{Menu, MenuBuilder, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter};

use crate::events;
use crate::manager;

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let icon = match app.default_window_icon() {
        Some(i) => i.clone(),
        None => return Ok(()),
    };
    TrayIconBuilder::with_id("tray")
        .icon(icon)
        .tooltip("浮匣 FloePod")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                manager::open_settings(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let settings = manager::current_settings(app);

    let open_settings = MenuItem::with_id(app, "open_settings", "设置", true, None::<&str>)?;
    let collect = MenuItem::with_id(
        app,
        "collect_clipboard",
        "收集剪贴板文字",
        true,
        None::<&str>,
    )?;
    let toggle_bars =
        MenuItem::with_id(app, "toggle_bars", "显示 / 隐藏全部匣", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出浮匣", true, None::<&str>)?;

    let mut menu = MenuBuilder::new(app).item(&open_settings);
    for pod in settings.pods.iter().filter(|p| p.enabled) {
        menu = menu.item(&MenuItem::with_id(
            app,
            format!("pod:{}", pod.id),
            format!("打开「{}」", pod.name),
            true,
            None::<&str>,
        )?);
    }
    menu = menu
        .item(&collect)
        .item(&toggle_bars)
        .separator()
        .item(&quit);
    menu.build()
}

pub fn refresh_menu(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id("tray") {
        if let Ok(menu) = build_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn on_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        "open_settings" => manager::open_settings(app),
        "toggle_bars" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                toggle_bars(&app);
            });
        }
        "collect_clipboard" => {
            if let Some(id) = crate::hotkeys::collect_into_first_pod(app) {
                let _ = app.emit_to(
                    events::pod_bar_label(id),
                    events::COLLECT_CLIPBOARD,
                    serde_json::json!({ "podId": id }),
                );
            }
        }
        "quit" => app.exit(0),
        id => {
            if let Some(id_str) = id.strip_prefix("pod:") {
                if let Ok(pid) = id_str.parse::<u64>() {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        manager::toggle_panel(&app, pid);
                    });
                }
            }
        }
    }
}

pub fn toggle_bars(app: &AppHandle) {
    let visible = manager::current_settings(app)
        .pods
        .iter()
        .filter(|p| p.enabled)
        .find_map(|p| manager::pod_bar(app, p.id))
        .map(|b| b.is_visible().unwrap_or(false))
        .unwrap_or(false);
    manager::set_all_bars(app, !visible);
}
