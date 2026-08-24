//! 浮匣 FloePod - 本地优先的屏幕边缘文件暂存工具（多匣版）。

mod autostart;
mod commands;
mod db;
mod events;
mod hotkeys;
mod lnk;
mod manager;
mod paths;
mod settings;
mod state;
mod tray;
mod watcher;
mod win;

use tauri::Manager;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run() {
    // 数据目录与数据库在窗口创建前准备好并注册，避免前端过早调用命令时 state 未就绪
    let data_dir = paths::resolve();
    let conn = match db::open(&data_dir) {
        Ok(c) => c,
        Err(e) => panic!("无法打开数据库: {e}"),
    };
    let app_state = state::AppState::new(conn, data_dir.clone());

    tauri::Builder::default()
        .manage(app_state)
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 二次启动：唤起已有实例
            manager::open_settings(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_drag::init())
        .setup(|app| {
            let settings = {
                let state = app.state::<state::AppState>();
                let conn = state.db.lock().unwrap();
                settings::load(&conn, &state.data_dir.to_string_lossy(), VERSION)?
            };

            tray::init(app.handle())?;
            watcher::spawn(app.handle().clone());

            commands::debug_log(&format!(
                "=== FloePod {VERSION} 启动 | 数据目录 {} | 匣 {} 个 | firstRunDone={} ===",
                settings.data_dir,
                settings.pods.len(),
                settings.first_run_done
            ));

            // 启动时显式校准系统自启动状态；失败不妨碍用户手动启动应用，但必须留痕。
            if let Err(e) = manager::sync_autostart(app.handle(), settings.autostart) {
                commands::debug_log(&format!("[autostart] {e}"));
            }

            // 落地：创建匣窗口 / 应用外观 / 监听 / 托盘
            manager::apply_settings(app.handle(), &settings);
            // 启动对账：把暂存文件夹中已有但未入库的文件读入列表
            app.state::<state::AppState>()
                .watcher_dirty
                .store(true, std::sync::atomic::Ordering::Relaxed);
            if let Err(e) = hotkeys::register(app.handle(), &settings) {
                eprintln!("[hotkeys] {e}");
            }
            manager::spawn_watchdog(app.handle().clone());

            // 首启（OOBE）或还没有任何匣：打开设置引导
            if !settings.first_run_done || settings.pods.is_empty() {
                manager::open_settings(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    "settings" => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    label if label.starts_with("pod_") => {
                        api.prevent_close();
                        // 面板被请求关闭（如 Alt+F4）-> 收起该匣面板
                        if let Some(id) = label
                            .strip_prefix("pod_")
                            .and_then(|s| s.strip_suffix("_panel"))
                            .and_then(|s| s.parse::<u64>().ok())
                        {
                            let app = window.app_handle().clone();
                            tauri::async_runtime::spawn(async move {
                                manager::hide_panel(&app, id);
                            });
                        }
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap,
            commands::get_pod,
            commands::get_monitors,
            commands::get_modifier_state,
            commands::get_hotkey_defaults,
            commands::create_pod,
            commands::update_pod,
            commands::delete_pod,
            commands::save_settings,
            commands::stage_paths,
            commands::stage_text,
            commands::list_pod_items,
            commands::remove_items,
            commands::prepare_drag_cut,
            commands::finalize_drag_cut,
            commands::cancel_drag_cut,
            commands::export_items,
            commands::read_thumbnail,
            commands::show_panel,
            commands::toggle_panel,
            commands::hide_panel,
            manager::get_panel_state,
            commands::set_panel_mode,
            commands::hold_pending_drop,
            commands::report_presence,
            commands::set_panel_pinned,
            commands::set_dragging_out,
            commands::set_pod_accept,
            commands::set_panel_size,
            commands::move_pod_bar,
            commands::toggle_all_bars,
            commands::open_settings,
            commands::log_frontend,
            commands::app_log,
            commands::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running FloePod");
}
