//! 识别“拖回自身”的应用内拖放。
//!
//! 剪切拖出（move）在目标接收后要清理暂存源文件；如果落点又是本应用自己的
//! 窗口（边缘浮动条 / 浮动面板），清理就会把用户刚拖出的文件删掉。拖拽插件
//! 只回报“已投递”，不告诉源窗口目标是谁，因此这里记录窗口级 Drop 事件与
//! 成功暂存的来源路径，供 `drag_out::finalize` 判定：只有真正被应用之外
//! （或另一个匣）接收的剪切才允许清理源文件。
//!
//! 记录在 Rust 侧完成：窗口 Drop 事件先于拖拽插件的会话结束回调到达，判定
//! 不依赖前端两个 WebView 之间的消息先后。

use std::path::Path;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::events::{self, PodWindow};
use crate::state::AppState;

/// 记录有效期。拖拽会话很短，几十秒足够覆盖 prepare → drop → finalize。
const RECORD_TTL: Duration = Duration::from_secs(90);

#[derive(Debug, Clone)]
pub struct DropRecord {
    /// 落点所属匣；应用内非匣窗口（设置等）为 None。
    pub pod_id: Option<u64>,
    /// 已规范化的路径键（`file_paths::path_key`）。
    pub paths: Vec<String>,
    pub at: Instant,
}

#[derive(Debug, Clone)]
pub struct RestageRecord {
    pub pod_id: u64,
    pub at: Instant,
}

/// 应用内落点归属。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InAppDropTarget {
    Pod(u64),
    OtherWindow,
}

/// 记录一次应用内窗口收到的文件拖放。
pub fn record_drop(app: &AppHandle, label: &str, paths: &[std::path::PathBuf]) {
    if paths.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    let pod_id = events::pod_window(label).map(|window| match window {
        PodWindow::Bar(id) | PodWindow::Panel(id) => id,
    });
    let now = Instant::now();
    let mut drops = state.recent_drops.lock().unwrap();
    drops.retain(|record| now.duration_since(record.at) < RECORD_TTL);
    drops.push(DropRecord {
        pod_id,
        paths: paths
            .iter()
            .map(|path| crate::file_paths::path_key(path))
            .collect(),
        at: now,
    });
}

/// 记录一次成功暂存：来源路径 -> 接收匣。落点是否真的收下了文件以此为准。
pub fn record_restages(state: &AppState, pod_id: u64, sources: &[std::path::PathBuf]) {
    if sources.is_empty() {
        return;
    }
    let now = Instant::now();
    let mut restages = state.recent_restages.lock().unwrap();
    restages.retain(|_, record| now.duration_since(record.at) < RECORD_TTL);
    for source in sources {
        restages.insert(
            crate::file_paths::path_key(source),
            RestageRecord { pod_id, at: now },
        );
    }
}

/// 该路径最近是否落在本应用的窗口里。
pub fn in_app_drop_target(state: &AppState, path: &Path) -> Option<InAppDropTarget> {
    let key = crate::file_paths::path_key(path);
    let now = Instant::now();
    let drops = state.recent_drops.lock().unwrap();
    drops
        .iter()
        .rev()
        .filter(|record| now.duration_since(record.at) < RECORD_TTL)
        .find(|record| record.paths.contains(&key))
        .map(|record| match record.pod_id {
            Some(pod_id) => InAppDropTarget::Pod(pod_id),
            None => InAppDropTarget::OtherWindow,
        })
}

/// 该路径最近是否被暂存进 `own_pod` 之外的某个匣（跨匣移动）。
pub fn restaged_into_other_pod(state: &AppState, path: &Path, own_pod: u64) -> bool {
    let key = crate::file_paths::path_key(path);
    let now = Instant::now();
    let restages = state.recent_restages.lock().unwrap();
    restages.get(&key).is_some_and(|record| {
        record.pod_id != own_pod && now.duration_since(record.at) < RECORD_TTL
    })
}
