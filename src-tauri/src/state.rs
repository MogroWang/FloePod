use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Mutex;
use std::time::Instant;

use notify::RecommendedWatcher;
use rusqlite::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelMode {
    #[default]
    List,
    Ask,
    Conflict,
}

impl PanelMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            PanelMode::List => "list",
            PanelMode::Ask => "ask",
            PanelMode::Conflict => "conflict",
        }
    }
}

/// 单个「匣」的运行时状态（看门狗 / 面板显隐）。
#[derive(Debug)]
pub struct PodRuntime {
    pub bar_inside: bool,
    pub panel_inside: bool,
    pub panel_visible: bool,
    pub panel_pinned: bool,
    /// 面板正在向外拖出文件（OLE 拖拽进行中）
    pub dragging_out: bool,
    pub mode: PanelMode,
    pub pending_drop: Vec<String>,
    /// 面板的 CSS 逻辑像素高度；设置原生窗口尺寸时再且只再乘一次 scale factor。
    pub panel_height: u32,
    pub last_change: Option<Instant>,
    /// 已应用的面板材质。None 表示尚未应用过，下一次 apply / show 时必然落地。
    /// 胶囊条材质已废弃（固定普通），不再跟踪。
    pub panel_material: Option<String>,
    /// 面板自动隐藏设置；apply_settings 时随配置刷新。
    pub auto_hide_enabled: bool,
    /// 鼠标离开后到自动隐藏的延迟（毫秒）。
    pub auto_hide_delay_ms: u64,
}

/// 剪切拖出开始时捕获的文件身份。稳定版 Rust 当前使用创建时间、写入时间、
/// 大小和类型做保守校验；文件系统 ID 字段为未来可用的按句柄身份信息预留。
/// 即使仍是同一个文件，只要拖拽期间内容发生变化也拒绝删除。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragCutFileIdentity {
    pub volume_serial_number: Option<u32>,
    pub file_index: Option<u64>,
    pub creation_time: u64,
    pub last_write_time: u64,
    pub size: u64,
    pub is_file: bool,
    pub is_dir: bool,
    /// 目录的确定性递归元数据摘要；普通文件为 `None`。
    pub tree_fingerprint: Option<u64>,
}

impl DragCutFileIdentity {
    pub fn matches(&self, current: &Self) -> bool {
        let primary_matches = match (
            self.volume_serial_number.zip(self.file_index),
            current.volume_serial_number.zip(current.file_index),
        ) {
            (Some(expected), Some(actual)) => expected == actual,
            _ => true,
        };
        primary_matches
            && self.creation_time == current.creation_time
            && self.last_write_time == current.last_write_time
            && self.size == current.size
            && self.is_file == current.is_file
            && self.is_dir == current.is_dir
            && self.tree_fingerprint == current.tree_fingerprint
    }
}

#[derive(Debug, Clone)]
pub struct DragCutEntry {
    pub item_id: i64,
    pub pod_id: i64,
    pub name: String,
    pub path: PathBuf,
    pub identity: DragCutFileIdentity,
}

#[derive(Debug)]
pub struct DragCutSnapshot {
    pub expires_at: Instant,
    pub entries: Vec<DragCutEntry>,
}

impl Default for PodRuntime {
    fn default() -> Self {
        Self {
            bar_inside: false,
            panel_inside: false,
            panel_visible: false,
            panel_pinned: false,
            dragging_out: false,
            mode: PanelMode::List,
            pending_drop: Vec::new(),
            panel_height: 0,
            last_change: None,
            panel_material: None,
            auto_hide_enabled: true,
            auto_hide_delay_ms: 320,
        }
    }
}

impl PodRuntime {
    /// 「单一活动面板」可以收起的普通面板。拖出与交互模式都必须受到保护。
    pub fn can_dismiss(&self) -> bool {
        self.panel_visible
            && !self.panel_pinned
            && !self.dragging_out
            && self.mode == PanelMode::List
    }

    /// 看门狗可以自动收起的状态。调用方必须在真正隐藏前、持锁时重新判断。
    pub fn can_auto_hide(&self, now: Instant, grace: std::time::Duration) -> bool {
        self.can_dismiss()
            && !self.bar_inside
            && !self.panel_inside
            && self
                .last_change
                .map(|changed| now.saturating_duration_since(changed) > grace)
                .unwrap_or(false)
    }

    /// 统一关闭转换，避免遗留 hidden+pinned、hidden+dragging 或陈旧 panel presence。
    pub fn mark_hidden(&mut self, now: Instant) {
        self.panel_visible = false;
        self.panel_pinned = false;
        self.panel_inside = false;
        self.dragging_out = false;
        self.mode = PanelMode::List;
        self.pending_drop.clear();
        self.last_change = Some(now);
    }
}

pub struct AppState {
    pub db: Mutex<Connection>,
    pub data_dir: PathBuf,
    /// pod_id -> 运行时状态
    pub pods: Mutex<HashMap<u64, PodRuntime>>,
    /// 串行化「运行态变更 + 原生窗口副作用」。
    ///
    /// 只锁 `pods` 无法阻止 show/hide 在释放状态锁后交叉执行，最终造成
    /// Win32 实际可见性与 `panel_visible` 分裂。所有 manager 面板转换都先持有此锁。
    pub panel_ops: Mutex<()>,
    /// 串行化“设置提交 -> 原生窗口 / watcher 落地”。只串行 DB 写入仍不够：
    /// 较旧命令可能在较新命令之后调用 apply_settings，把运行态回放成旧快照。
    pub settings_ops: Mutex<()>,
    /// 串行化会同时触碰暂存文件与条目索引的操作。
    ///
    /// 选名、文件 I/O、SQLite 入库和 watcher 对账必须位于同一个临界区；
    /// 否则并发暂存可能选中同名目标，watcher 也可能把半成品提前写入数据库。
    pub file_ops: Mutex<()>,
    /// 一次性剪切拖出令牌。令牌在 finalize 时先消费；取消或超时后也不能复用。
    pub drag_cut_tokens: Mutex<HashMap<String, DragCutSnapshot>>,
    pub next_drag_cut_token: AtomicU64,
    /// 用户的“显示 / 隐藏全部匣”选择。隐藏是临时暂停，不应销毁 pin/Ask 上下文。
    pub bars_visible: AtomicBool,
    /// 「自动屏蔽」配置的内存快照（apply_settings 时刷新），供轮询线程低频读取，
    /// 避免轮询线程反复读库。
    pub auto_block_enabled: AtomicBool,
    pub auto_block_apps: Mutex<Vec<String>>,
    /// 进入屏蔽前匣的可见性；解除屏蔽后据此恢复，不覆盖用户的手动隐藏。
    pub auto_block_restore: AtomicBool,
    /// 最近一次应用自身文件写入。必须使用单调时钟；系统墙钟回拨不能让 watcher
    /// 永久停留在“刚写入”的抑制窗口。
    pub last_stage: Mutex<Option<Instant>>,
    /// 暂存文件夹监听的脏标记（有文件变化待对账）
    pub watcher_dirty: AtomicBool,
    /// pod_id -> 暂存文件夹监听器
    pub watcher: Mutex<HashMap<u64, RecommendedWatcher>>,
}

impl AppState {
    pub fn new(db: Connection, data_dir: PathBuf) -> Self {
        Self {
            db: Mutex::new(db),
            data_dir,
            pods: Mutex::new(HashMap::new()),
            panel_ops: Mutex::new(()),
            settings_ops: Mutex::new(()),
            file_ops: Mutex::new(()),
            drag_cut_tokens: Mutex::new(HashMap::new()),
            next_drag_cut_token: AtomicU64::new(1),
            bars_visible: AtomicBool::new(true),
            auto_block_enabled: AtomicBool::new(false),
            auto_block_apps: Mutex::new(Vec::new()),
            auto_block_restore: AtomicBool::new(false),
            last_stage: Mutex::new(None),
            watcher_dirty: AtomicBool::new(false),
            watcher: Mutex::new(HashMap::new()),
        }
    }

    pub fn mark_staged(&self) {
        *self.last_stage.lock().unwrap() = Some(Instant::now());
    }

    pub fn staged_recently(&self) -> bool {
        self.last_stage
            .lock()
            .unwrap()
            .map(|last| last.elapsed() < std::time::Duration::from_secs(3))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn dismiss_protects_pinned_modal_and_dragging_panels() {
        let mut runtime = PodRuntime {
            panel_visible: true,
            ..PodRuntime::default()
        };
        assert!(runtime.can_dismiss());

        runtime.panel_pinned = true;
        assert!(!runtime.can_dismiss());
        runtime.panel_pinned = false;

        runtime.mode = PanelMode::Ask;
        assert!(!runtime.can_dismiss());
        runtime.mode = PanelMode::Conflict;
        assert!(!runtime.can_dismiss());
        runtime.mode = PanelMode::List;

        runtime.dragging_out = true;
        assert!(!runtime.can_dismiss());
    }

    #[test]
    fn auto_hide_requires_presence_timestamp_and_grace() {
        let now = Instant::now();
        let mut runtime = PodRuntime {
            panel_visible: true,
            ..PodRuntime::default()
        };
        assert!(!runtime.can_auto_hide(now, Duration::from_millis(320)));

        runtime.last_change = Some(now - Duration::from_millis(321));
        assert!(runtime.can_auto_hide(now, Duration::from_millis(320)));

        runtime.panel_inside = true;
        assert!(!runtime.can_auto_hide(now, Duration::from_millis(320)));
    }

    #[test]
    fn hiding_restores_runtime_invariants() {
        let now = Instant::now();
        let mut runtime = PodRuntime {
            panel_inside: true,
            panel_visible: true,
            panel_pinned: true,
            dragging_out: true,
            mode: PanelMode::Ask,
            pending_drop: vec!["a.txt".into()],
            ..PodRuntime::default()
        };

        runtime.mark_hidden(now);

        assert!(!runtime.panel_visible);
        assert!(!runtime.panel_pinned);
        assert!(!runtime.panel_inside);
        assert!(!runtime.dragging_out);
        assert_eq!(runtime.mode, PanelMode::List);
        assert!(runtime.pending_drop.is_empty());
        assert_eq!(runtime.last_change, Some(now));
    }
}
