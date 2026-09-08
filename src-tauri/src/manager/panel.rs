//! Serialized panel transitions. State mutation and native effects share panel_ops.
use super::material::{apply_panel_material_if_changed, refresh_pod_bar_chrome};
use super::windows::{place_panel, pod_bar, pod_panel};
use super::{current_settings, pod_of};
use crate::settings::{Pod, Settings};
use crate::state::{AppState, PanelMode, PodRuntime};
use crate::{events, win};
use serde::Serialize;
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
const PANEL_FADE_OUT_MS: u64 = 220;
#[derive(Debug, Clone, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PanelSnapshot {
    #[schemars(extend("enum" = ["list","ask","conflict"]))]
    pub mode: String,
    pub paths: Vec<String>,
    pub pinned: bool,
    pub visible: bool,
    pub dragging_out: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PanelToggleAction {
    /// 全局暂停时，toggle 的首要含义是恢复 UI；若目标原本未打开则同时打开并固定。
    Resume { show_target: bool },
    /// 浮动面板未显示：以固定方式弹出，保持到再次点击匣或主动取消固定。
    ShowPinned,
    /// 浮动面板已在显示：点击匣直接收起。
    Hide,
}

pub(super) fn panel_toggle_action(
    bars_visible: bool,
    runtime: Option<&PodRuntime>,
) -> PanelToggleAction {
    let panel_visible = runtime
        .map(|runtime| runtime.panel_visible)
        .unwrap_or(false);
    if !bars_visible {
        return PanelToggleAction::Resume {
            show_target: !panel_visible,
        };
    }
    if panel_visible {
        PanelToggleAction::Hide
    } else {
        PanelToggleAction::ShowPinned
    }
}

pub fn panel_snapshot(app: &AppHandle, id: u64) -> PanelSnapshot {
    let state = app.state::<AppState>();
    let guard = state.pods.lock().unwrap();
    let r = guard.get(&id);
    PanelSnapshot {
        mode: r
            .map(|runtime| runtime.mode)
            .unwrap_or(PanelMode::List)
            .as_str()
            .to_string(),
        paths: r
            .map(|runtime| runtime.pending_drop.clone())
            .unwrap_or_default(),
        pinned: r.map(|runtime| runtime.panel_pinned).unwrap_or(false),
        visible: r.map(|runtime| runtime.panel_visible).unwrap_or(false),
        dragging_out: r.map(|runtime| runtime.dragging_out).unwrap_or(false),
    }
}

/// 从同一份运行态快照同时同步模式、固定状态以及完整状态事件，避免前端半更新。
/// 调用方必须持有 `AppState::panel_ops`。
pub(super) fn emit_panel_snapshot(app: &AppHandle, id: u64) {
    let snapshot = panel_snapshot(app, id);
    if pod_panel(app, id).is_none() {
        return;
    }
    let label = events::pod_panel_label(id);
    let _ = events::PANEL_MODE.emit_to(
        app,
        &label,
        crate::events::ModeChanged {
            mode: snapshot.mode.clone(),
            paths: snapshot.paths.clone(),
        },
    );
    let _ = events::PANEL_PINNED.emit_to(
        app,
        &label,
        crate::events::PinnedChanged {
            pinned: snapshot.pinned,
        },
    );
    let _ = events::PANEL_STATE.emit_to(app, label, snapshot);
}

pub fn set_panel_mode(app: &AppHandle, id: u64, mode: &str) -> Result<(), String> {
    let mode = match mode {
        "list" => PanelMode::List,
        "ask" => PanelMode::Ask,
        "conflict" => PanelMode::Conflict,
        other => return Err(format!("未知浮动面板模式: {other}")),
    };
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    {
        let mut guard = state.pods.lock().unwrap();
        let runtime = guard.entry(id).or_default();
        if mode == PanelMode::Ask && runtime.pending_drop.is_empty() {
            return Err("询问模式缺少待处理路径".into());
        }
        runtime.mode = mode;
        if mode != PanelMode::Ask {
            runtime.pending_drop.clear();
        }
    }
    emit_panel_snapshot(app, id);
    Ok(())
}

/// 保存一批待询问路径并在同一个窗口操作临界区内显示浮动面板。
///
/// 已有 Ask 时合并而不是覆盖；Conflict 必须先完成，避免丢失任一交互上下文。
pub fn hold_pending_drop(app: &AppHandle, id: u64, paths: Vec<String>) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    // 浮动面板会把这些路径原样展示；提前挡掉相对路径等畸形输入，
    // 真正的文件校验仍由 stage_paths 完成。
    for path in &paths {
        if !std::path::Path::new(path).is_absolute() {
            return Err(format!("待暂存路径必须是绝对路径: {path}"));
        }
    }
    let pod = pod_of(app, id).ok_or_else(|| format!("匣 {id} 不存在或已停用"))?;
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    if !state.bars_visible.load(Ordering::Relaxed) {
        return Err("浮匣当前已全部隐藏".into());
    }
    {
        let mut guard = state.pods.lock().unwrap();
        let runtime = guard.entry(id).or_default();
        if runtime.mode == PanelMode::Conflict {
            return Err("请先处理当前导出冲突".into());
        }
        for path in paths {
            if !runtime.pending_drop.contains(&path) {
                runtime.pending_drop.push(path);
            }
        }
        runtime.mode = PanelMode::Ask;
    }
    if !show_panel_locked(app, id, &pod, false) {
        return Err(format!("无法显示匣 {id} 的浮动面板"));
    }
    Ok(())
}

/// 前端上报的是 CSS 逻辑像素；这里只保存逻辑高度，`place_panel` 统一缩放一次。
pub fn set_panel_size(app: &AppHandle, id: u64, height: u32) {
    // 先读配置再进入 panel_ops，统一锁顺序为 db -> panel_ops，避免
    // 与 set_all_bars/apply_settings 并发时反向加锁。
    let pod = pod_of(app, id);
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    let resize_now = {
        let mut guard = state.pods.lock().unwrap();
        let runtime = guard.entry(id).or_default();
        let height = height.clamp(160, 900);
        let changed = runtime.panel_height != height;
        runtime.panel_height = height;
        changed && runtime.panel_visible
    };
    if resize_now {
        if let Some(pod) = pod.as_ref() {
            place_panel(app, pod);
        }
    }
}

/// 延迟隐藏浮动面板窗口，给前端留出淡出动画的时间窗。
/// 调用前提：transition_to_hidden_locked 已把运行态置为隐藏。
/// 延迟期间浮动面板可能被重新显示（指针重新悬停 / 主动弹出），
/// 任务执行时按运行态自检，一旦 panel_visible 回到 true 就放弃隐藏，
/// 避免「浮动面板刚淡入又被藏掉」。
pub(super) fn schedule_delayed_panel_hide(app: &AppHandle, id: u64) {
    if let Some(runtime) = app.state::<AppState>().pods.lock().unwrap().get_mut(&id) {
        runtime.panel_hide_at = Some(Instant::now() + Duration::from_millis(PANEL_FADE_OUT_MS));
    }
}

pub(super) fn finish_delayed_hides(app: &AppHandle, now: Instant) {
    let ids: Vec<_> = app
        .state::<AppState>()
        .pods
        .lock()
        .unwrap()
        .iter_mut()
        .filter_map(|(id, runtime)| runtime.take_due_hide(now).then_some(*id))
        .collect();
    for id in ids {
        hide_panel_window(app, id);
    }
}

/// 调用方必须持有 `AppState::panel_ops`。
pub(super) fn transition_to_hidden_locked<F>(app: &AppHandle, id: u64, predicate: F) -> bool
where
    F: FnOnce(&PodRuntime) -> bool,
{
    let changed = {
        let state = app.state::<AppState>();
        let mut guard = state.pods.lock().unwrap();
        let runtime = guard.entry(id).or_default();
        if !predicate(runtime) {
            false
        } else {
            runtime.mark_hidden(Instant::now());
            true
        }
    };
    if !changed {
        return false;
    }

    // 运行态转换与原生副作用由 panel_ops 串行化；不会再出现旧 show 在新 hide 后补显。
    // 原生窗口不立即隐藏：先让前端播放淡出动画，延迟任务自检后再 SW_HIDE。
    schedule_delayed_panel_hide(app, id);
    // 必须用 emit_to 定向发送：`emit` 是全局广播，会让其他仍可见的浮动面板
    // 也收到 PANEL_HIDDEN 并把 DOM 置为透明（pre-show）。
    if pod_panel(app, id).is_some() {
        let _ = events::PANEL_HIDDEN.emit_to(app, events::pod_panel_label(id), ());
    }
    emit_panel_snapshot(app, id);
    true
}

/// 仅暂停原生窗口，不改变运行态、也不发送 `PANEL_HIDDEN`。
///
/// `PANEL_HIDDEN` 表示真正的浮动面板状态转换，前端收到后会清空 Ask/Conflict
/// 上下文。“隐藏全部匣”只是全局暂停，必须无损保留这些上下文。
/// 调用方必须持有 `AppState::panel_ops`。
pub(super) fn hide_panel_window(app: &AppHandle, id: u64) {
    if let Some(panel) = pod_panel(app, id) {
        if let Ok(hwnd) = panel.hwnd() {
            win::hide_window(hwnd.0 as isize);
        }
    }
}

/// 单一活动浮动面板：收起除 id 外所有「可见、未固定、列表模式」的浮动面板。
/// 固定（panel_pinned）以及正在拖入询问/冲突解决（mode != List）的浮动面板不受影响。
/// 关闭了自动隐藏的浮动面板等价于固定，同样不被隐藏；OLE 拖出中的浮动面板也不受影响。
/// 走 transition_to_hidden_locked 的淡出路径，新浮动面板弹出与旧浮动面板淡出重叠不闪烁。
/// 调用方必须持有 `AppState::panel_ops`。
pub(super) fn dismiss_other_panels_locked(app: &AppHandle, id: u64) {
    let state = app.state::<AppState>();
    let others: Vec<u64> = {
        let guard = state.pods.lock().unwrap();
        guard.keys().copied().filter(|pid| *pid != id).collect()
    };
    for pid in others {
        // 逐项在真正隐藏前重新判断，不使用可能已经过期的候选快照。
        transition_to_hidden_locked(app, pid, |runtime| {
            runtime.auto_hide_enabled && runtime.can_dismiss()
        });
    }
}

/// 调用方必须持有 `AppState::panel_ops`。
pub(super) fn show_panel_locked(app: &AppHandle, id: u64, pod: &Pod, pin_on_show: bool) -> bool {
    let Some(panel) = pod_panel(app, id) else {
        return false;
    };
    let Ok(hwnd) = panel.hwnd() else {
        return false;
    };

    let state = app.state::<AppState>();
    let was_visible = {
        let mut guard = state.pods.lock().unwrap();
        let runtime = guard.entry(id).or_default();
        if pin_on_show {
            runtime.panel_pinned = true;
        }
        runtime.panel_visible
    };

    // 已可见时只校正原生窗口并重发快照，不再次收起其他浮动面板。
    // 这也避免 hold_pending_drop 后的重复 show 干扰刚刚打开的另一个匣。
    if !was_visible {
        dismiss_other_panels_locked(app, id);
    }
    place_panel(app, pod);
    // 材质只在变化时重设；正常由 apply_settings 落地，这里兜底检测一次。
    apply_panel_material_if_changed(app, pod);
    let _ = panel.set_title(&format!("{} 浮动面板", pod.name));
    win::prefer_rounded_corners(hwnd.0 as isize);
    // 只压制 1px 外描边与焦点过渡；不动样式位、不重刷框架——面板的
    // 系统阴影依赖 tao 常驻的框架样式位（见 suppress_panel_frame 注释）。
    win::suppress_panel_frame(hwnd.0 as isize);
    win::show_no_activate(hwnd.0 as isize);
    // 显示兄弟浮动面板本身也可能触发透明 WebView 的非客户区合成回归。
    refresh_pod_bar_chrome(app, id);

    {
        let mut guard = state.pods.lock().unwrap();
        let runtime = guard.entry(id).or_default();
        runtime.panel_visible = true;
        runtime.panel_hide_at = None;
        if pin_on_show {
            runtime.panel_pinned = true;
        }
        if !was_visible && !runtime.bar_inside && !runtime.panel_inside {
            // 非 presence 入口（例如拖入完成后弹出）也必须最终可被看门狗收起。
            runtime.last_change = Some(Instant::now());
        }
    }
    if !was_visible {
        let _ = events::PANEL_SHOWN.emit_to(app, events::pod_panel_label(id), ());
    }
    emit_panel_snapshot(app, id);
    true
}

pub fn show_panel(app: &AppHandle, id: u64) {
    let state = app.state::<AppState>();
    // 全局隐藏期间忽略来自旧 hover timer / 拖放事件的普通 show；托盘/快捷键
    // 走 toggle_panel，仍可显式打开一个固定浮动面板。
    if !state.bars_visible.load(Ordering::Relaxed) {
        return;
    }
    let Some(pod) = pod_of(app, id) else { return };
    let _operation = state.panel_ops.lock().unwrap();
    if !state.bars_visible.load(Ordering::Relaxed) {
        return;
    }
    show_panel_locked(app, id, &pod, false);
}

pub fn hide_panel(app: &AppHandle, id: u64) {
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    // 全局暂停期间只可能收到暂停前在途的 hide/动画回调；
    // 它不应销毁为恢复而保留的 pin/Ask/Conflict 状态。
    if !state.bars_visible.load(Ordering::Relaxed) {
        return;
    }
    transition_to_hidden_locked(app, id, |_| true);
}

pub fn toggle_panel(app: &AppHandle, id: u64) {
    // 用一次配置快照同时解决目标匣与全局恢复，并保持 db -> panel_ops
    // 的唯一锁顺序。
    let settings = current_settings(app);
    let pod = settings
        .pods
        .iter()
        .find(|pod| pod.id == id && pod.enabled)
        .cloned();
    let Some(pod) = pod else { return };
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    let action = {
        let guard = state.pods.lock().unwrap();
        panel_toggle_action(state.bars_visible.load(Ordering::Relaxed), guard.get(&id))
    };

    match action {
        PanelToggleAction::Resume { show_target } => {
            set_all_bars_locked(app, &settings, true);
            if show_target {
                show_panel_locked(app, id, &pod, true);
            }
        }
        PanelToggleAction::ShowPinned => {
            show_panel_locked(app, id, &pod, true);
        }
        PanelToggleAction::Hide => {
            transition_to_hidden_locked(app, id, |_| true);
        }
    }
}

pub fn set_panel_pinned(app: &AppHandle, id: u64, pinned: bool) {
    let Some(pod) = pod_of(app, id) else { return };
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    if !state.bars_visible.load(Ordering::Relaxed) {
        // 暂停后才到达的 pin 请求不得把原生浮动面板重新显示。
        emit_panel_snapshot(app, id);
        return;
    }
    if pinned {
        let visible = state
            .pods
            .lock()
            .unwrap()
            .get(&id)
            .map(|r| r.panel_visible)
            .unwrap_or(false);
        if !visible {
            show_panel_locked(app, id, &pod, true);
        } else {
            state
                .pods
                .lock()
                .unwrap()
                .entry(id)
                .or_default()
                .panel_pinned = true;
            emit_panel_snapshot(app, id);
        }
    } else {
        state
            .pods
            .lock()
            .unwrap()
            .entry(id)
            .or_default()
            .panel_pinned = false;
        emit_panel_snapshot(app, id);
    }
}

pub fn set_dragging_out(app: &AppHandle, id: u64, dragging: bool) {
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    if !state.bars_visible.load(Ordering::Relaxed) {
        return;
    }
    state
        .pods
        .lock()
        .unwrap()
        .entry(id)
        .or_default()
        .dragging_out = dragging;
}

pub fn report_presence(app: &AppHandle, id: u64, window: &str, inside: bool) {
    let state = app.state::<AppState>();
    if !state.bars_visible.load(Ordering::Relaxed) {
        return;
    }
    let _operation = state.panel_ops.lock().unwrap();
    if !state.bars_visible.load(Ordering::Relaxed) {
        return;
    }
    let visible = {
        let mut guard = state.pods.lock().unwrap();
        let r = guard.entry(id).or_default();
        match window {
            "bar" => r.bar_inside = inside,
            // 隐藏之后延迟到达的 pointerenter 不能复活陈旧 panel presence。
            "panel" => r.panel_inside = inside && r.panel_visible,
            _ => return,
        }
        r.last_change = Some(Instant::now());
        r.panel_visible
    };
    // 指针进入本匣：若本匣浮动面板可见，收起其他未固定浮动面板，维持单一活动浮动面板
    // （否则「B 收起中、指针回到 A」的路径会让 A、B 同时显示）。
    if inside && visible {
        dismiss_other_panels_locked(app, id);
    }
}

/// 调用方必须持有 `AppState::panel_ops`。
pub(super) fn set_all_bars_locked(app: &AppHandle, settings: &Settings, visible: bool) {
    let state = app.state::<AppState>();
    state.bars_visible.store(visible, Ordering::Relaxed);
    for pod in settings.pods.iter().filter(|p| p.enabled) {
        if let Some(bar) = pod_bar(app, pod.id) {
            if let Ok(hwnd) = bar.hwnd() {
                if visible {
                    // 显示路径自带非客户区刷新：WebView2 在透明无边框窗口
                    // 显示时可能重新显露幽灵标题栏，不能依赖后续焦点事件。
                    win::show_bar_no_activate(hwnd.0 as isize);
                } else {
                    win::hide_window(hwnd.0 as isize);
                }
            }
        }
        if visible {
            let panel_was_open = {
                let mut guard = state.pods.lock().unwrap();
                let runtime = guard.entry(pod.id).or_default();
                // 暂停时已清掉 presence；恢复时重启离开宽限，避免未固定
                // 列表浮动面板因旧时间戳在第一个 watchdog tick 立刻消失。
                if runtime.panel_visible {
                    runtime.last_change = Some(Instant::now());
                }
                runtime.panel_visible
            };
            if panel_was_open {
                show_panel_locked(app, pod.id, pod, false);
            }
        } else {
            {
                let mut guard = state.pods.lock().unwrap();
                let runtime = guard.entry(pod.id).or_default();
                runtime.bar_inside = false;
                runtime.panel_inside = false;
                runtime.dragging_out = false;
                runtime.last_change = Some(Instant::now());
            }
            // 全局暂停不是 panel 的状态转换：只隐藏原生窗口，
            // 不发 PANEL_HIDDEN，否则前端会清空 pin/Ask/Conflict 上下文。
            hide_panel_window(app, pod.id);
        }
    }
}

/// 显示 / 隐藏全部匣。“隐藏”是可逆的全局暂停，不销毁浮动面板状态。
pub fn set_all_bars(app: &AppHandle, visible: bool) {
    // 始终先取 DB 快照再取 panel_ops，与 toggle/size/pin 保持一致锁顺序。
    let settings = current_settings(app);
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    set_all_bars_locked(app, &settings, visible);
}
