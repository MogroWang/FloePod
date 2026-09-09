//! 浮匣 FloePod - 本地优先的屏幕边缘文件暂存工具（多匣版）。

mod autostart;
mod clipboard;
mod commands;
mod db;
mod drag_out;
mod events;
mod export;
mod file_fingerprint;
mod file_ops;
mod file_paths;
mod handoff;
mod hotkeys;
mod lifecycle;
mod lnk;
mod logging;
mod manager;
mod menu;
mod operations;
mod parser_worker;
mod paths;
mod pods;
mod policy;
mod privacy;
mod rules;
mod search;
mod security;
mod settings;
mod shell_integration;
mod shutdown;
mod staging;
mod state;
mod thumbnail;
mod tray;
mod watcher;
mod win;

use tauri::Manager;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run() {
    if parser_worker::run_if_requested() {
        return;
    }
    // 数据目录与数据库在窗口创建前准备好并注册，避免前端过早调用命令时 state 未就绪
    let data_dir = paths::resolve();
    let conn = match db::open(&data_dir) {
        Ok(c) => c,
        Err(e) => panic!("无法打开数据库: {e}"),
    };
    let app_state = state::AppState::new(conn, data_dir.clone());

    tauri::Builder::default()
        .manage(app_state)
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // 资源管理器菜单通过二次启动把文件复制投递到第一个已解锁匣；
            // 普通二次启动仍唤起设置窗口。
            if !shell_integration::handle_args(app, argv) {
                manager::open_settings(app);
            }
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
                match settings::load(&conn, &state.data_dir.to_string_lossy(), VERSION) {
                    Ok(settings) => settings,
                    Err(error) => {
                        // 损坏的设置不能让应用无法启动：保留数据库原始值并回退默认设置
                        //（表现为重新走首次引导），运行中的读取路径仍保持严格报错。
                        crate::logging::write(&format!(
                            "[settings] 启动读取设置失败，已回退默认设置: {error}"
                        ));
                        settings::Settings::default()
                    }
                }
            };

            security::ensure_configured(&settings);
            // 先恢复被中断的跨盘移动并执行保留策略，再启动 watcher，避免对账抢先。
            staging::recover_pending_moves(app.handle());
            security::retention::start(app.handle())?;
            tray::init(app.handle())?;
            watcher::spawn(app.handle().clone());

            logging::write(&format!(
                "=== FloePod {VERSION} 启动 | 数据目录 {} | 匣 {} 个 | firstRunDone={} ===",
                settings.data_dir,
                settings.pods.len(),
                settings.first_run_done
            ));

            // 启动时显式校准系统自启动状态；失败不妨碍用户手动启动应用，但必须留痕。
            if let Err(e) = manager::sync_autostart(app.handle(), settings.autostart) {
                logging::write(&format!("[autostart] {e}"));
            }

            // 落地：创建匣窗口 / 应用外观 / 监听 / 托盘
            manager::apply_settings(app.handle(), &settings);
            // 启动对账：把暂存文件夹中已有但未入库的文件读入列表
            app.state::<state::AppState>()
                .watcher_dirty
                .store(true, std::sync::atomic::Ordering::Relaxed);
            if let Err(e) = hotkeys::register(app.handle(), &settings) {
                logging::write(&format!("[hotkeys] {e}"));
            }
            manager::spawn_watchdog(app.handle().clone());
            // 自动屏蔽轮询依赖 apply_settings 写入的内存快照，必须在之后启动。
            manager::spawn_auto_block_watcher(app.handle().clone());

            let initial_args = std::env::args().collect::<Vec<_>>();
            let _ = shell_integration::handle_args(app.handle(), initial_args);

            // 首启（OOBE）或还没有任何匣：打开设置引导
            if !settings.first_run_done || settings.pods.is_empty() {
                manager::open_settings(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 焦点变化时幂等重放浮动面板材质（亚克力恒定全量下发，不随焦点
            // 降级）；边缘浮动条无材质，仅顺带清理非客户区样式。非匣窗口的
            // 材质为空，refresh_window_material 内部会直接返回。
            if let tauri::WindowEvent::Focused(_) = event {
                let label = window.label();
                if events::pod_window(label).is_some() {
                    manager::refresh_window_material(window.app_handle(), label);
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    "settings" => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    label if events::pod_window(label).is_some() => {
                        api.prevent_close();
                        // 浮动面板被请求关闭（如 Alt+F4）-> 收起该匣浮动面板
                        if let Some(events::PodWindow::Panel(id)) = events::pod_window(label) {
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
        .invoke_handler(include!(concat!(env!("OUT_DIR"), "/invoke-handler.rs")))
        .build(tauri::generate_context!())
        .expect("error while building FloePod")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                shutdown::handle(app, code, api);
            }
        });
}
