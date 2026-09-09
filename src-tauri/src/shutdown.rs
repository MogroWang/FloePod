//! Join background effects while the UI loop can still service pending native window requests.
use crate::state::AppState;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

pub fn handle(app: &AppHandle, code: Option<i32>, api: tauri::ExitRequestApi) {
    let state = app.state::<AppState>();
    if state.shutdown_finished.load(Ordering::Acquire) {
        return;
    }
    api.prevent_exit();
    if state.shutdown_started.swap(true, Ordering::AcqRel) {
        return;
    }
    state.tasks.close();
    for task in [
        &state.retention_task,
        &state.watchdog_task,
        &state.auto_block_task,
        &state.reconcile_task,
    ] {
        task.request_stop();
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        for task in [
            &state.retention_task,
            &state.watchdog_task,
            &state.auto_block_task,
            &state.reconcile_task,
        ] {
            task.stop();
        }
        state.tasks.wait();
        state.watcher.lock().unwrap().clear();
        state.unlocked_pods.lock().unwrap().clear();
        state.drag_cut_tokens.lock().unwrap().clear();
        state.shutdown_finished.store(true, Ordering::Release);
        app.exit(code.unwrap_or(0));
    });
}
