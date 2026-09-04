//! 窗口编排：多「匣」窗口的创建与摆放、面板显隐（不抢焦点 + 看门狗动画收起）。

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::events;
use crate::settings::{Pod, Settings};
use crate::state::{AppState, PanelMode, PodRuntime};
use crate::win;

/// 匣（胶囊条）长边；短边（贴屏幕边缘一侧）来自每个匣的 bar_width 设置。
const POD_BAR_LONG: u32 = 190;
/// 拖入接纳态：短条在此基础上加宽为圆角矩形
const POD_BAR_ACCEPT_GROW: u32 = 18;
const PANEL_GAP: u32 = 10;
const PANEL_MARGIN: u32 = 8;
/// 自动屏蔽轮询间隔；前台进程切换的感知延迟上限。
const AUTO_BLOCK_POLL: Duration = Duration::from_millis(600);

#[derive(Debug, Clone, Copy, PartialEq)]
struct MonitorGeometry {
    rect: (i32, i32, i32, i32),
    scale_factor: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub name: String,
    pub label: String,
    pub primary: bool,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PanelSnapshot {
    pub mode: String,
    pub paths: Vec<String>,
    pub pinned: bool,
    pub visible: bool,
    pub dragging_out: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelToggleAction {
    /// 全局暂停时，toggle 的首要含义是恢复 UI；若目标原本未打开则同时打开并固定。
    Resume { show_target: bool },
    /// 面板未显示：以固定方式弹出，保持到再次点击匣或主动取消固定。
    ShowPinned,
    /// 面板已在显示：点击匣直接收起。
    Hide,
}

fn panel_toggle_action(bars_visible: bool, runtime: Option<&PodRuntime>) -> PanelToggleAction {
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

/// 严格版设置读取：加载 + 完整校验，供 watcher 等必须拿到可信配置的路径使用。
pub fn load_validated_settings(app: &AppHandle) -> Result<Settings, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let settings = crate::settings::load(
        &conn,
        &state.data_dir.to_string_lossy(),
        env!("CARGO_PKG_VERSION"),
    )?;
    crate::settings::validate(&settings, &state.data_dir.to_string_lossy())?;
    Ok(settings)
}

pub fn current_settings(app: &AppHandle) -> Settings {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    match crate::settings::load(
        &conn,
        &state.data_dir.to_string_lossy(),
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(settings) => settings,
        Err(error) => {
            crate::logging::write(&format!("[settings] 读取当前设置失败: {error}"));
            Settings::default()
        }
    }
}

fn pod_of(app: &AppHandle, id: u64) -> Option<Pod> {
    current_settings(app)
        .pods
        .into_iter()
        .find(|pod| pod.id == id && pod.enabled)
}

pub fn pod_bar(app: &AppHandle, id: u64) -> Option<WebviewWindow> {
    app.get_webview_window(&events::pod_bar_label(id))
}

pub fn pod_panel(app: &AppHandle, id: u64) -> Option<WebviewWindow> {
    app.get_webview_window(&events::pod_panel_label(id))
}

fn pods_guard<'a>(
    state: &'a State<'_, AppState>,
) -> std::sync::MutexGuard<'a, HashMap<u64, PodRuntime>> {
    state.pods.lock().unwrap()
}

/// 找到匣所在显示器：按名称匹配，空名或未找到回退主显示器。
///
/// Tauri 的显示器位置/尺寸是物理像素；同时返回目标显示器的缩放率，避免
/// 面板窗口尚在旧显示器时误用 `panel.scale_factor()` 计算新位置和尺寸。
fn monitor(app: &AppHandle, pod: &Pod) -> Option<MonitorGeometry> {
    let monitors = app.available_monitors().ok()?;
    if !pod.monitor.is_empty() {
        for m in &monitors {
            if m.name().map(|s| s.as_str()) == Some(pod.monitor.as_str()) {
                let size = m.size();
                let pos = m.position();
                return Some(MonitorGeometry {
                    rect: (pos.x, pos.y, size.width as i32, size.height as i32),
                    scale_factor: m.scale_factor(),
                });
            }
        }
    }
    let m = app.primary_monitor().ok().flatten()?;
    let size = m.size();
    let pos = m.position();
    Some(MonitorGeometry {
        rect: (pos.x, pos.y, size.width as i32, size.height as i32),
        scale_factor: m.scale_factor(),
    })
}

pub fn list_monitors(app: &AppHandle) -> Vec<MonitorInfo> {
    let Some(monitors) = app.available_monitors().ok() else {
        return vec![];
    };
    let primary = app.primary_monitor().ok().flatten();
    let mut entries: Vec<(bool, i32, i32, &tauri::Monitor)> = monitors
        .iter()
        .map(|m| {
            let is_primary = primary
                .as_ref()
                .map(|p| p.name() == m.name())
                .unwrap_or(false);
            let position = m.position();
            (is_primary, position.x, position.y, m)
        })
        .collect();
    // 主显示器排第一，其余按从左到右、从上到下排序，保证「显示器 N」编号稳定。
    entries.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    let mut out = Vec::new();
    for (idx, (is_primary, _, _, m)) in entries.into_iter().enumerate() {
        // 下拉里空名称选项已表示「主显示器」，逐个显示器改用带编号的标签，
        // 避免列表中同时出现两个「主显示器」无法区分。
        let label = if is_primary {
            format!("显示器 {}（主）", idx + 1)
        } else {
            format!("显示器 {}", idx + 1)
        };
        let position = m.position();
        let size = m.size();
        out.push(MonitorInfo {
            name: m.name().map(|name| name.as_str()).unwrap_or("").to_string(),
            label,
            primary: is_primary,
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
            scale_factor: m.scale_factor(),
        });
    }
    out
}

/// 胶囊条窗口的几何（长边方向由边缘决定）。短边来自匣的 bar_width 设置。
fn bar_geometry_for_monitor(
    monitor: (i32, i32, i32, i32),
    edge: &str,
    offset: f64,
    accepting: bool,
    scale: f64,
    bar_width: u32,
) -> (i32, i32, i32, i32) {
    let (mx, my, mw, mh) = monitor;
    let short_value = if accepting {
        bar_width.saturating_add(POD_BAR_ACCEPT_GROW)
    } else {
        bar_width
    };
    let short = scale_logical_px(short_value, scale);
    let long = scale_logical_px(POD_BAR_LONG, scale);
    let vertical = matches!(edge, "left" | "right");
    let (w, h) = if vertical {
        (short, long)
    } else {
        (long, short)
    };
    let (x, y) = match edge {
        "right" => (
            mx + mw - w,
            my + (mh as f64 * offset).round() as i32 - h / 2,
        ),
        "bottom" => (
            mx + (mw as f64 * offset).round() as i32 - w / 2,
            my + mh - h,
        ),
        "top" => (mx + (mw as f64 * offset).round() as i32 - w / 2, my),
        _ => (mx, my + (mh as f64 * offset).round() as i32 - h / 2),
    };
    let max_y = (my + mh - h).max(my);
    let y = y.clamp(my, max_y);
    let max_x = (mx + mw - w).max(mx);
    let x = x.clamp(mx, max_x);
    (x, y, w, h)
}

fn bar_geometry(app: &AppHandle, pod: &Pod, accepting: bool) -> Option<(i32, i32, i32, i32)> {
    // (x, y, w, h)，物理像素
    let target = monitor(app, pod)?;
    Some(bar_geometry_for_monitor(
        target.rect,
        &pod.edge,
        pod.offset,
        accepting,
        target.scale_factor,
        pod.bar_width,
    ))
}

pub fn place_pod_bar(app: &AppHandle, pod: &Pod, accepting: bool) {
    let Some(bar) = pod_bar(app, pod.id) else {
        return;
    };
    let Some((x, y, w, h)) = bar_geometry(app, pod, accepting) else {
        return;
    };
    let _ = bar.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = bar.set_position(PhysicalPosition::new(x, y));
    // 胶囊条材质已废弃（固定普通半透明），无需按材质裁剪区域；
    // 以 radius=0 清除历史残留的区域，并顺带幂等清理非客户区样式。
    if let Ok(hwnd) = bar.hwnd() {
        win::set_bar_region(hwnd.0 as isize, w, h, 0, &pod.edge);
    }
}

/// 把请求的面板矩形限制在显示器工作矩形内。
///
/// 先限制宽高再 clamp 坐标，保证在高 DPI、窄屏或异常尺寸下也不会出现
/// `min > max` 导致 Rust `clamp` panic。
fn scale_logical_px(value: u32, scale: f64) -> i32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (value as f64 * scale).round().clamp(1.0, i32::MAX as f64) as i32
}

fn panel_geometry(
    monitor: (i32, i32, i32, i32),
    bar: (i32, i32, i32, i32),
    edge: &str,
    requested_width: i32,
    requested_height: i32,
    scale: f64,
) -> (i32, i32, i32, i32) {
    let (mx, my, mw, mh) = monitor;
    let (bx, by, bw, bh) = bar;
    let gap = scale_logical_px(PANEL_GAP, scale);
    let margin = scale_logical_px(PANEL_MARGIN, scale);
    let available_width = mw.saturating_sub(margin * 2).max(1);
    let available_height = mh.saturating_sub(margin * 2).max(1);
    let width = requested_width.max(1).min(available_width);
    let height = requested_height.max(1).min(available_height);

    let (raw_x, raw_y) = match edge {
        "right" => (bx - gap - width, by + bh / 2 - height / 2),
        "bottom" => (bx + bw / 2 - width / 2, by - gap - height),
        "top" => (bx + bw / 2 - width / 2, by + bh + gap),
        _ => (bx + bw + gap, by + bh / 2 - height / 2),
    };

    let min_x = mx.saturating_add(margin);
    let min_y = my.saturating_add(margin);
    let max_x = mx
        .saturating_add(mw)
        .saturating_sub(margin)
        .saturating_sub(width)
        .max(min_x);
    let max_y = my
        .saturating_add(mh)
        .saturating_sub(margin)
        .saturating_sub(height)
        .max(min_y);
    (
        raw_x.clamp(min_x, max_x),
        raw_y.clamp(min_y, max_y),
        width,
        height,
    )
}

/// 面板：贴着匣弹出，长边方向垂直/水平时对齐到匣中心。
fn place_panel(app: &AppHandle, pod: &Pod) {
    let Some(panel) = pod_panel(app, pod.id) else {
        return;
    };
    let state = app.state::<AppState>();
    // panel_height 为 0（前端尚未上报）时用默认值：否则会按最小高度显示，
    // 待前端上报后再 resize，造成「显示后跳一下」的闪烁。
    let logical_height = {
        let guard = state.pods.lock().unwrap();
        guard
            .get(&pod.id)
            .map(|r| r.panel_height)
            .filter(|&h| h > 0)
            .unwrap_or(420)
    };
    let Some(target) = monitor(app, pod) else {
        return;
    };
    let scale = target.scale_factor;
    let requested_width = scale_logical_px(pod.panel_width, scale);
    let requested_height = scale_logical_px(logical_height, scale);

    let monitor_rect = target.rect;
    let bar_rect = bar_geometry_for_monitor(
        monitor_rect,
        &pod.edge,
        pod.offset,
        false,
        target.scale_factor,
        pod.bar_width,
    );
    let (x, y, width, height) = panel_geometry(
        monitor_rect,
        bar_rect,
        &pod.edge,
        requested_width,
        requested_height.max(scale_logical_px(120, scale)),
        scale,
    );
    let _ = panel.set_size(PhysicalSize::new(width as u32, height as u32));
    let _ = panel.set_position(PhysicalPosition::new(x, y));
}

fn ensure_pod_windows(app: &AppHandle, pod: &Pod) {
    // 运行态与启用的匣同生命周期，不能等到第一次悬停才临时创建。
    // 否则首笔 hold_pending_drop / set_panel_size 会因 get_mut(None) 被静默丢弃。
    app.state::<AppState>()
        .pods
        .lock()
        .unwrap()
        .entry(pod.id)
        .or_default();

    let bar_label = events::pod_bar_label(pod.id);
    let panel_label = events::pod_panel_label(pod.id);
    if app.get_webview_window(&bar_label).is_none() {
        // 胶囊条不设置标题文字：即使非客户区渲染被系统事件短暂恢复，
        // 也不会显示出「匣名称」标题栏。
        if let Err(err) =
            WebviewWindowBuilder::new(app, &bar_label, tauri::WebviewUrl::App("index.html".into()))
                .title("")
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .shadow(false)
                .focusable(true) // 必须可聚焦才能接收拖放事件
                .visible(false)
                .build()
        {
            crate::logging::write(&format!("[window] 创建 {bar_label} 失败: {err}"));
        }
    }
    if app.get_webview_window(&panel_label).is_none() {
        if let Err(err) = WebviewWindowBuilder::new(
            app,
            &panel_label,
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title(format!("{} 面板", pod.name))
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(true)
        .focusable(true) // 必须可聚焦才能接收拖放事件
        .visible(false)
        .build()
        {
            crate::logging::write(&format!("[window] 创建 {panel_label} 失败: {err}"));
        }
    }
    // 胶囊条形状由前端自绘：禁用 Windows 11 系统窗口圆角，
    // 否则 DWM 圆角会把贴边的圆角矩形裁掉，看起来「显示不全」。
    if let Some(bar) = pod_bar(app, pod.id) {
        if let Ok(hwnd) = bar.hwnd() {
            win::disable_rounding(hwnd.0 as isize);
            // 材质需要用窗口区域（region）裁剪成胶囊形状：先清掉样式里残留的
            // 标题栏位并关闭 DWM 非客户区渲染，否则区域会让系统画出标题栏。
            win::prepare_shaped_window(hwnd.0 as isize);
        }
    }
    // 面板：请求系统圆角，与 CSS 的 clip-path 圆角轮廓对齐
    if let Some(panel) = pod_panel(app, pod.id) {
        if let Ok(hwnd) = panel.hwnd() {
            win::prefer_rounded_corners(hwnd.0 as isize);
        }
    }
    place_pod_bar(app, pod, false);
}

fn destroy_pod_windows(app: &AppHandle, id: u64) {
    let labels = [events::pod_bar_label(id), events::pod_panel_label(id)];
    for l in labels {
        if let Some(w) = app.get_webview_window(&l) {
            let _ = w.destroy();
        }
    }
}

/// 让所有窗口与当前设置中的匣对齐：创建缺失、销毁多余的。
fn sync_pods_with_settings(app: &AppHandle, s: &Settings) {
    let wanted: HashMap<u64, &Pod> = s
        .pods
        .iter()
        .filter(|p| p.enabled)
        .map(|p| (p.id, p))
        .collect();

    let existing: std::collections::HashSet<u64> = app
        .webview_windows()
        .keys()
        .filter_map(|label| match events::pod_window(label) {
            Some(events::PodWindow::Bar(id) | events::PodWindow::Panel(id)) => Some(id),
            None => None,
        })
        .collect();

    for id in existing {
        if !wanted.contains_key(&id) {
            destroy_pod_windows(app, id);
            app.state::<AppState>().pods.lock().unwrap().remove(&id);
        }
    }

    for pod in s.pods.iter().filter(|p| p.enabled) {
        ensure_pod_windows(app, pod);
    }
}

/// 面板云母 / 普通材质：tauri 的 set_effects（Win11 22H2+ 走 DWM systembackdrop）。
/// 面板是矩形窗口 + 系统圆角，systembackdrop 与 DWM 圆角轮廓天然对齐；
/// 云母不随窗口焦点移除，可以放心使用。
fn apply_material(window: &WebviewWindow, material: &str) -> bool {
    use tauri::window::{Effect, EffectsBuilder};
    let effect = match material {
        // 云母仅在 Windows 11 可用；不支持的系统上 set_effects 返回 Err，
        // 忽略后表现为普通半透明。
        "mica" => Some(Effect::Mica),
        _ => None,
    };
    match effect {
        Some(effect) => {
            let config = EffectsBuilder::new().effects([effect]).build();
            window.set_effects(Some(config)).is_ok()
        }
        None => window.set_effects(None).is_ok(),
    }
}

/// 透明浮层材质落地：供匣面板与右键菜单共用，按材质分流到不同系统机制。
///
/// 亚克力走 SWCA ACCENT（见 win::apply_panel_acrylic）：DWM 系统背景
/// （tauri set_effects 的 systembackdrop）在窗口失焦后移除整个 backdrop，
/// 而面板免激活显示、绝大多数时间不持有焦点，是「不聚焦就看不到材质」
/// 的根源。云母无此限制（失焦不移除），仍走 tauri set_effects；普通即
/// 无系统材质。任何分支都先清掉 ACCENT 与 DWM effect 两个通道，避免
/// 云母切到亚克力时旧 systembackdrop 与新 ACCENT 叠加。
pub(crate) fn apply_window_material(window: &WebviewWindow, material: &str) -> bool {
    let Ok(hwnd) = window.hwnd() else {
        return false;
    };
    win::disable_accent(hwnd.0 as isize);
    let _ = apply_material(window, "plain");
    match material {
        "acrylic" => win::apply_panel_acrylic(hwnd.0 as isize),
        "mica" => apply_material(window, "mica"),
        _ => true,
    }
}

/// 强制刷新指定匣胶囊条的无边框状态。透明 WebView2 的幽灵标题栏可能由
/// 兄弟面板或右键菜单获得焦点触发，因此不能只在胶囊条自身收到焦点事件时修复。
pub(crate) fn refresh_pod_bar_chrome(app: &AppHandle, id: u64) {
    if let Some(bar) = pod_bar(app, id) {
        if let Ok(hwnd) = bar.hwnd() {
            win::prepare_shaped_window(hwnd.0 as isize);
        }
    }
}

/// 焦点变化时幂等重放面板材质：亚克力与云母各自重发一次全量材质，
/// 保证无论面板是否持有焦点，材质属性始终处于已下发状态。
/// 胶囊条材质已废弃（固定普通），仅顺带幂等清理非客户区样式，
/// 杜绝任何来源恢复的标题栏样式位驻留。
pub fn refresh_window_material(app: &AppHandle, label: &str) {
    let target = match events::pod_window(label) {
        Some(events::PodWindow::Bar(id)) => pod_bar(app, id).map(|window| (id, window, true)),
        Some(events::PodWindow::Panel(id)) => pod_panel(app, id).map(|window| (id, window, false)),
        None => None,
    };
    let Some((id, window, is_bar)) = target else {
        return;
    };
    // 任一匣窗口的焦点变化都同步刷新其胶囊条。用户点击面板时胶囊条通常
    // 不会收到新的 Focused 事件，但 WebView2 仍可能重绘它的幽灵标题栏。
    refresh_pod_bar_chrome(app, id);
    if is_bar {
        return;
    }
    let material = {
        let state = app.state::<AppState>();
        let guard = state.pods.lock().unwrap();
        guard
            .get(&id)
            .and_then(|runtime| runtime.panel_material.clone())
    };
    match material.as_deref() {
        Some("acrylic") => {
            if let Ok(hwnd) = window.hwnd() {
                let _ = win::apply_panel_acrylic(hwnd.0 as isize);
            }
        }
        Some("mica") => {
            let _ = apply_material(&window, "mica");
        }
        _ => {}
    }
}

/// 把与运行态相关的配置同步进 PodRuntime（自动隐藏设置）。
/// 面板材质不在这里处理：必须对隐藏的面板也落地，见 apply_panel_material_if_changed。
fn sync_runtime_config(app: &AppHandle, pod: &Pod) {
    let state = app.state::<AppState>();
    let mut guard = state.pods.lock().unwrap();
    let runtime = guard.entry(pod.id).or_default();
    runtime.auto_hide_enabled = pod.auto_hide;
    runtime.auto_hide_delay_ms = pod.auto_hide_delay_ms;
}

/// 面板材质只在变化时重设（每次显示都重设亚克力会引起重绘闪烁）。
///
/// 必须对隐藏 / 未固定的面板同样生效：此前只在面板可见时应用，而运行态
/// 已经记录了新材质，显示路径的变化检测就再也不触发，导致「改材质时
/// 面板没固定」永远不生效。
fn apply_panel_material_if_changed(app: &AppHandle, pod: &Pod) {
    let changed = {
        let state = app.state::<AppState>();
        let mut guard = state.pods.lock().unwrap();
        let runtime = guard.entry(pod.id).or_default();
        if runtime.panel_material.as_deref() == Some(pod.panel_material.as_str()) {
            false
        } else {
            runtime.panel_material = Some(pod.panel_material.clone());
            true
        }
    };
    if changed {
        if let Some(panel) = pod_panel(app, pod.id) {
            let _ = apply_window_material(&panel, &pod.panel_material);
        }
    }
}

fn panel_snapshot(app: &AppHandle, id: u64) -> PanelSnapshot {
    let state = app.state::<AppState>();
    let guard = pods_guard(&state);
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

/// 面板 WebView 挂载后主动拉取一次，弥补窗口首次加载前发送的事件不会排队的问题。
#[tauri::command]
pub async fn get_panel_state(app: AppHandle, pod_id: u64) -> PanelSnapshot {
    panel_snapshot(&app, pod_id)
}

/// 从同一份运行态快照同时同步模式、固定状态以及完整状态事件，避免前端半更新。
/// 调用方必须持有 `AppState::panel_ops`。
fn emit_panel_snapshot(app: &AppHandle, id: u64) {
    let snapshot = panel_snapshot(app, id);
    if pod_panel(app, id).is_none() {
        return;
    }
    let label = events::pod_panel_label(id);
    let _ = app.emit_to(
        &label,
        events::PANEL_MODE,
        serde_json::json!({ "mode": snapshot.mode.clone(), "paths": snapshot.paths.clone() }),
    );
    let _ = app.emit_to(
        &label,
        events::PANEL_PINNED,
        serde_json::json!({ "pinned": snapshot.pinned }),
    );
    let _ = app.emit_to(label, events::PANEL_STATE, snapshot);
}

pub fn set_panel_mode(app: &AppHandle, id: u64, mode: &str) -> Result<(), String> {
    let mode = match mode {
        "list" => PanelMode::List,
        "ask" => PanelMode::Ask,
        "conflict" => PanelMode::Conflict,
        other => return Err(format!("未知面板模式: {other}")),
    };
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    {
        let mut guard = pods_guard(&state);
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

/// 保存一批待询问路径并在同一个窗口操作临界区内显示面板。
///
/// 已有 Ask 时合并而不是覆盖；Conflict 必须先完成，避免丢失任一交互上下文。
pub fn hold_pending_drop(app: &AppHandle, id: u64, paths: Vec<String>) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    // 面板会把这些路径原样展示；提前挡掉相对路径等畸形输入，
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
        let mut guard = pods_guard(&state);
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
        return Err(format!("无法显示匣 {id} 的面板"));
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
        let mut guard = pods_guard(&state);
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

/// 面板淡出动画的时长；后端在此之后才真正隐藏原生窗口。
/// 必须与 PodPanel.vue 的 .panel-fade-out 动画时长保持一致。
const PANEL_FADE_OUT_MS: u64 = 220;

/// 延迟隐藏面板窗口，给前端留出淡出动画的时间窗。
/// 调用前提：transition_to_hidden_locked 已把运行态置为隐藏。
/// 延迟期间面板可能被重新显示（指针重新悬停 / 主动弹出），
/// 任务执行时按运行态自检，一旦 panel_visible 回到 true 就放弃隐藏，
/// 避免「面板刚淡入又被藏掉」。
fn schedule_delayed_panel_hide(app: &AppHandle, id: u64) {
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(PANEL_FADE_OUT_MS));
        let state = handle.state::<AppState>();
        let _operation = state.panel_ops.lock().unwrap();
        let visible = state
            .pods
            .lock()
            .unwrap()
            .get(&id)
            .map(|runtime| runtime.panel_visible)
            .unwrap_or(false);
        if !visible {
            hide_panel_window(&handle, id);
        }
    });
}

/// 调用方必须持有 `AppState::panel_ops`。
fn transition_to_hidden_locked<F>(app: &AppHandle, id: u64, predicate: F) -> bool
where
    F: FnOnce(&PodRuntime) -> bool,
{
    let changed = {
        let state = app.state::<AppState>();
        let mut guard = pods_guard(&state);
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
    // 必须用 emit_to 定向发送：`emit` 是全局广播，会让其他仍可见的面板
    // 也收到 PANEL_HIDDEN 并把 DOM 置为透明（pre-show）。
    if pod_panel(app, id).is_some() {
        let _ = app.emit_to(events::pod_panel_label(id), events::PANEL_HIDDEN, ());
    }
    emit_panel_snapshot(app, id);
    true
}

/// 仅暂停原生窗口，不改变运行态、也不发送 `PANEL_HIDDEN`。
///
/// `PANEL_HIDDEN` 表示真正的面板状态转换，前端收到后会清空 Ask/Conflict
/// 上下文。“隐藏全部匣”只是全局暂停，必须无损保留这些上下文。
/// 调用方必须持有 `AppState::panel_ops`。
fn hide_panel_window(app: &AppHandle, id: u64) {
    if let Some(panel) = pod_panel(app, id) {
        if let Ok(hwnd) = panel.hwnd() {
            win::hide_window(hwnd.0 as isize);
        }
    }
}

/// 单一活动面板：收起除 id 外所有「可见、未固定、列表模式」的面板。
/// 固定（panel_pinned）以及正在拖入询问/冲突解决（mode != List）的面板不受影响。
/// 关闭了自动隐藏的面板等价于固定，同样不被隐藏；OLE 拖出中的面板也不受影响。
/// 走 transition_to_hidden_locked 的淡出路径，新面板弹出与旧面板淡出重叠不闪烁。
/// 调用方必须持有 `AppState::panel_ops`。
fn dismiss_other_panels_locked(app: &AppHandle, id: u64) {
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
fn show_panel_locked(app: &AppHandle, id: u64, pod: &Pod, pin_on_show: bool) -> bool {
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

    // 已可见时只校正原生窗口并重发快照，不再次收起其他面板。
    // 这也避免 hold_pending_drop 后的重复 show 干扰刚刚打开的另一个匣。
    if !was_visible {
        dismiss_other_panels_locked(app, id);
    }
    place_panel(app, pod);
    // 材质只在变化时重设；正常由 apply_settings 落地，这里兜底检测一次。
    apply_panel_material_if_changed(app, pod);
    let _ = panel.set_title(&format!("{} 面板", pod.name));
    win::prefer_rounded_corners(hwnd.0 as isize);
    win::show_no_activate(hwnd.0 as isize);
    // 显示兄弟面板本身也可能触发透明 WebView 的非客户区合成回归。
    refresh_pod_bar_chrome(app, id);

    {
        let mut guard = state.pods.lock().unwrap();
        let runtime = guard.entry(id).or_default();
        runtime.panel_visible = true;
        if pin_on_show {
            runtime.panel_pinned = true;
        }
        if !was_visible && !runtime.bar_inside && !runtime.panel_inside {
            // 非 presence 入口（例如拖入完成后弹出）也必须最终可被看门狗收起。
            runtime.last_change = Some(Instant::now());
        }
    }
    if !was_visible {
        let _ = app.emit_to(events::pod_panel_label(id), events::PANEL_SHOWN, ());
    }
    emit_panel_snapshot(app, id);
    true
}

pub fn show_panel(app: &AppHandle, id: u64) {
    let state = app.state::<AppState>();
    // 全局隐藏期间忽略来自旧 hover timer / 拖放事件的普通 show；托盘/快捷键
    // 走 toggle_panel，仍可显式打开一个固定面板。
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
        let guard = pods_guard(&state);
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
        // 暂停后才到达的 pin 请求不得把原生面板重新显示。
        emit_panel_snapshot(app, id);
        return;
    }
    if pinned {
        let visible = pods_guard(&state)
            .get(&id)
            .map(|r| r.panel_visible)
            .unwrap_or(false);
        if !visible {
            show_panel_locked(app, id, &pod, true);
        } else {
            pods_guard(&state).entry(id).or_default().panel_pinned = true;
            emit_panel_snapshot(app, id);
        }
    } else {
        pods_guard(&state).entry(id).or_default().panel_pinned = false;
        emit_panel_snapshot(app, id);
    }
}

pub fn set_dragging_out(app: &AppHandle, id: u64, dragging: bool) {
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    if !state.bars_visible.load(Ordering::Relaxed) {
        return;
    }
    pods_guard(&state).entry(id).or_default().dragging_out = dragging;
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
        let mut guard = pods_guard(&state);
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
    // 指针进入本匣：若本匣面板可见，收起其他未固定面板，维持单一活动面板
    // （否则「B 收起中、指针回到 A」的路径会让 A、B 同时显示）。
    if inside && visible {
        dismiss_other_panels_locked(app, id);
    }
}

/// 拖入接纳：短条变为圆角矩形（窗口加宽），结束后收回。
pub fn set_pod_accept(app: &AppHandle, id: u64, accepting: bool) {
    let Some(pod) = pod_of(app, id) else { return };
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    if !state.bars_visible.load(Ordering::Relaxed) {
        return;
    }
    place_pod_bar(app, &pod, accepting);
}

/// 拖动胶囊条过程中实时重定位（不写库）；松手后由 update_pod 持久化 offset。
pub fn move_pod_bar(app: &AppHandle, id: u64, offset: f64) {
    let Some(mut pod) = pod_of(app, id) else {
        return;
    };
    pod.offset = offset.clamp(0.0, 1.0);
    place_pod_bar(app, &pod, false);
}

/// 看门狗：逐个匣检查--面板未固定、未在拖出、列表模式且指针离开超过宽限期 -> 淡出隐藏。
/// 单一活动面板由 show_panel / report_presence 主动维持；这里只负责指针离开后的兜底隐藏。
/// 关闭了「自动隐藏」的匣不参与自动隐藏，面板保持到用户手动关闭。
pub fn spawn_watchdog(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(100));
        let state = app.state::<AppState>();
        if !state.bars_visible.load(Ordering::Relaxed) {
            continue;
        }
        let _operation = state.panel_ops.lock().unwrap();
        if !state.bars_visible.load(Ordering::Relaxed) {
            continue;
        }
        let ids: Vec<u64> = state.pods.lock().unwrap().keys().copied().collect();
        for id in ids {
            let now = Instant::now();
            transition_to_hidden_locked(&app, id, |runtime| {
                runtime.auto_hide_enabled
                    && runtime.can_auto_hide(now, Duration::from_millis(runtime.auto_hide_delay_ms))
            });
        }
    });
}
pub fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 调用方必须持有 `AppState::panel_ops`。
fn set_all_bars_locked(app: &AppHandle, settings: &Settings, visible: bool) {
    let state = app.state::<AppState>();
    state.bars_visible.store(visible, Ordering::Relaxed);
    for pod in settings.pods.iter().filter(|p| p.enabled) {
        if let Some(bar) = pod_bar(app, pod.id) {
            if let Ok(hwnd) = bar.hwnd() {
                if visible {
                    win::show_no_activate(hwnd.0 as isize);
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
                // 列表面板因旧时间戳在第一个 watchdog tick 立刻消失。
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

/// 显示 / 隐藏全部匣。“隐藏”是可逆的全局暂停，不销毁面板状态。
pub fn set_all_bars(app: &AppHandle, visible: bool) {
    // 始终先取 DB 快照再取 panel_ops，与 toggle/size/pin 保持一致锁顺序。
    let settings = current_settings(app);
    let state = app.state::<AppState>();
    let _operation = state.panel_ops.lock().unwrap();
    set_all_bars_locked(app, &settings, visible);
}

/// 同步系统开机自启动状态。调用方负责在持久化设置时把此副作用纳入事务回滚。
pub fn sync_autostart(_app: &AppHandle, enabled: bool) -> Result<(), String> {
    crate::autostart::sync(enabled).map_err(|e| {
        format!(
            "{}开机自启动失败: {e}",
            if enabled { "启用" } else { "禁用" }
        )
    })
}

/// 设置落地：同步匣窗口、材质、监听、托盘全量应用。
/// 自启动属于可失败的系统副作用，由保存设置和启动流程显式调用 `sync_autostart`。
pub fn apply_settings(app: &AppHandle, s: &Settings) {
    // 自动屏蔽配置驻留内存：轮询线程每几百毫秒读取一次，不能每次都查库。
    {
        let state = app.state::<AppState>();
        state
            .auto_block_enabled
            .store(s.auto_block.enabled, Ordering::Relaxed);
        *state.auto_block_apps.lock().unwrap() = s.auto_block.apps.clone();
    }

    // 窗口创建/销毁与所有面板显隐串行。对已固定、可见的面板立即应用新的
    // monitor/edge/offset/panelWidth/material，而不是等到关闭重开。
    {
        let state = app.state::<AppState>();
        let _operation = state.panel_ops.lock().unwrap();
        sync_pods_with_settings(app, s);

        for pod in s.pods.iter().filter(|p| p.enabled) {
            sync_runtime_config(app, pod);
            // 面板材质无论可见与否都要落地，否则未固定的面板改材质永远不生效。
            apply_panel_material_if_changed(app, pod);
            place_pod_bar(app, pod, false);
            if let Some(panel) = pod_panel(app, pod.id) {
                let _ = panel.set_title(&format!("{} 面板", pod.name));
            }
            let panel_visible = state
                .pods
                .lock()
                .unwrap()
                .get(&pod.id)
                .map(|runtime| runtime.panel_visible)
                .unwrap_or(false);
            if panel_visible {
                place_panel(app, pod);
                emit_panel_snapshot(app, pod.id);
            }
        }
    }

    // 暂存文件夹监听（每个匣一个）
    crate::watcher::restart_all(app);

    // 配置完成（OOBE 结束）后亮相
    if s.first_run_done
        && !s.pods.is_empty()
        && app.state::<AppState>().bars_visible.load(Ordering::Relaxed)
    {
        for pod in s.pods.iter().filter(|p| p.enabled) {
            if let Some(bar) = pod_bar(app, pod.id) {
                if let Ok(hwnd) = bar.hwnd() {
                    win::show_no_activate(hwnd.0 as isize);
                }
            }
        }
    }

    crate::tray::refresh_menu(app);
    let _ = app.emit(events::SETTINGS_CHANGED, s.clone());
}

/// 进程名匹配：取文件名部分，忽略大小写、可选的 .exe 后缀与首尾引号。
/// 允许用户粘贴完整路径或只写进程名，两种写法都能命中。
fn exe_matches(candidate: &str, foreground: &str) -> bool {
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
/// 屏蔽走 set_all_bars 的全局暂停语义：面板的固定 / 询问 / 冲突上下文得以保留，
/// 恢复时原样回归。进入屏蔽前记录可见性，解除时只恢复屏蔽前可见的状态，
/// 不会覆盖用户在屏蔽期间手动执行的「隐藏全部匣」。
pub fn spawn_auto_block_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let mut was_blocked = false;
        loop {
            std::thread::sleep(AUTO_BLOCK_POLL);
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
                continue;
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
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_inside_monitor(rect: (i32, i32, i32, i32), monitor: (i32, i32, i32, i32)) {
        let (x, y, width, height) = rect;
        let (mx, my, mw, mh) = monitor;
        assert!(width > 0 && height > 0);
        assert!(x >= mx);
        assert!(y >= my);
        assert!(x.saturating_add(width) <= mx.saturating_add(mw));
        assert!(y.saturating_add(height) <= my.saturating_add(mh));
    }

    #[test]
    fn logical_panel_size_is_scaled_exactly_once() {
        assert_eq!(scale_logical_px(420, 1.0), 420);
        assert_eq!(scale_logical_px(420, 1.5), 630);
        assert_eq!(scale_logical_px(420, 2.0), 840);
        assert_eq!(scale_logical_px(420, f64::NAN), 420);
    }

    #[test]
    fn oversized_panel_is_clipped_without_invalid_clamp_bounds() {
        let monitor = (0, 0, 1080, 1080);
        let bar = (0, 445, 44, 190);
        let rect = panel_geometry(monitor, bar, "left", 2080, 1800, 1.0);
        assert_eq!(rect.2, 1064);
        assert_eq!(rect.3, 1064);
        assert_inside_monitor(rect, monitor);
    }

    #[test]
    fn panel_geometry_handles_negative_monitor_origins_on_every_edge() {
        let monitor = (-1920, -200, 1920, 1080);
        let bars = [
            ("left", (-1920, 245, 44, 190)),
            ("right", (-44, 245, 44, 190)),
            ("top", (-1055, -200, 190, 44)),
            ("bottom", (-1055, 836, 190, 44)),
        ];
        for (edge, bar) in bars {
            assert_inside_monitor(panel_geometry(monitor, bar, edge, 570, 630, 1.0), monitor);
        }
    }

    #[test]
    fn bar_geometry_scales_all_logical_dimensions_for_target_monitor() {
        let monitor = (1920, 0, 3840, 2160);
        let normal = bar_geometry_for_monitor(monitor, "right", 0.5, false, 2.0, 44);
        let accepting = bar_geometry_for_monitor(monitor, "right", 0.5, true, 2.0, 44);

        assert_eq!(normal, (5672, 890, 88, 380));
        assert_eq!(accepting, (5636, 890, 124, 380));
        assert_inside_monitor(normal, monitor);
        assert_inside_monitor(accepting, monitor);

        // 自定义匣宽度同样只缩放一次，接纳态在此基础上加宽固定值。
        let wide = bar_geometry_for_monitor(monitor, "left", 0.5, false, 2.0, 60);
        assert_eq!(wide, (1920, 890, 120, 380));
        let wide_accepting = bar_geometry_for_monitor(monitor, "left", 0.5, true, 2.0, 60);
        assert_eq!(wide_accepting, (1920, 890, 156, 380));
    }

    #[test]
    fn panel_gap_and_margin_scale_with_target_monitor() {
        let monitor = (0, 0, 1920, 1080);
        let bar = (0, 350, 88, 380);
        let rect = panel_geometry(monitor, bar, "left", 840, 840, 2.0);

        assert_eq!(rect, (108, 120, 840, 840));

        let oversized = panel_geometry(monitor, bar, "left", 4000, 4000, 2.0);
        assert_eq!(oversized, (16, 16, 1888, 1048));
        assert_inside_monitor(oversized, monitor);
    }

    #[test]
    fn toggle_during_global_pause_restores_instead_of_closing_logically_open_panel() {
        let runtime = PodRuntime {
            panel_visible: true,
            panel_pinned: true,
            mode: PanelMode::Conflict,
            ..PodRuntime::default()
        };

        assert_eq!(
            panel_toggle_action(false, Some(&runtime)),
            PanelToggleAction::Resume { show_target: false }
        );
        assert_eq!(
            panel_toggle_action(false, None),
            PanelToggleAction::Resume { show_target: true }
        );
        assert_eq!(
            panel_toggle_action(true, Some(&runtime)),
            PanelToggleAction::Hide
        );
    }

    #[test]
    fn foreground_exe_matching_is_name_based_and_case_insensitive() {
        assert!(exe_matches("game.exe", "GAME.EXE"));
        assert!(exe_matches("C:\\Games\\game.exe", "game.exe"));
        assert!(exe_matches("game", "game.exe"));
        assert!(exe_matches("\"My Game.exe\"", "my game.exe"));
        assert!(!exe_matches("game.exe", "other.exe"));
        assert!(!exe_matches("game.exe", "gamelite.exe"));
        assert!(!exe_matches("", "game.exe"));
        assert!(!exe_matches("   ", "game.exe"));
    }

    #[test]
    fn monitor_info_keeps_the_frontend_camel_case_contract() {
        let monitor = MonitorInfo {
            name: "DISPLAY2".into(),
            label: "显示器 2".into(),
            primary: false,
            x: -1920,
            y: 0,
            width: 3840,
            height: 2160,
            scale_factor: 2.0,
        };
        let value = serde_json::to_value(monitor).unwrap();
        assert_eq!(value["scaleFactor"], 2.0);
        assert_eq!(value["width"], 3840);
        assert!(value.get("scale_factor").is_none());
    }
}
