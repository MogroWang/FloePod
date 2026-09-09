//! Foreground application policy, independent from panel rendering.
use super::panel::set_all_bars;
use crate::{state::AppState, win};
use std::{sync::atomic::Ordering, time::Duration};
use tauri::{AppHandle, Manager};
const AUTO_BLOCK_POLL: Duration = Duration::from_millis(600);
/// 进程名匹配：取文件名部分，忽略大小写、可选的 .exe 后缀与首尾引号。
/// 允许用户粘贴完整路径或只写进程名，两种写法都能命中。
pub(super) fn exe_matches(candidate: &str, foreground: &str) -> bool {
    let base = |raw: &str| {
        let trimmed = raw.trim().trim_matches('"');
        let name = std::path::Path::new(trimmed)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| trimmed.to_string());
        name.to_lowercase()
    };
    let candidate = base(candidate);
    if candidate.is_empty() {
        return false;
    }
    let foreground = base(foreground);
    fn strip_exe(s: &str) -> &str {
        s.strip_suffix(".exe").unwrap_or(s)
    }
    strip_exe(&candidate) == strip_exe(&foreground)
}

/// 自动屏蔽轮询：配置的应用位于前台时暂时隐藏全部匣，切回其他应用后自动恢复。
///
/// 屏蔽走 set_all_bars 的全局暂停语义：浮动面板的固定 / 询问 / 冲突上下文得以保留，
/// 恢复时原样回归。进入屏蔽前记录可见性，解除时只恢复屏蔽前可见的状态，
/// 不会覆盖用户在屏蔽期间手动执行的「隐藏全部匣」。
pub fn spawn_auto_block_watcher(app: AppHandle) {
    let owner = app.clone();
    let mut was_blocked = false;
    if let Err(error) =
        owner
            .state::<AppState>()
            .auto_block_task
            .start("auto-block", AUTO_BLOCK_POLL, move |_| {
                let state = app.state::<AppState>();
                let apps = state.auto_block_apps.lock().unwrap().clone();
                let blocked = state.auto_block_enabled.load(Ordering::Relaxed)
                    && !apps.is_empty()
                    && win::foreground_exe()
                        .map(|foreground| {
                            apps.iter()
                                .any(|candidate| exe_matches(candidate, &foreground))
                        })
                        .unwrap_or(false);
                if blocked == was_blocked {
                    return Ok(());
                }
                was_blocked = blocked;
                if blocked {
                    state.auto_block_restore.store(
                        state.bars_visible.load(Ordering::Relaxed),
                        Ordering::Relaxed,
                    );
                    set_all_bars(&app, false);
                } else if state.auto_block_restore.load(Ordering::Relaxed) {
                    set_all_bars(&app, true);
                }
                Ok(())
            })
    {
        crate::logging::write(&format!("[auto-block] {error}"));
    }
}
