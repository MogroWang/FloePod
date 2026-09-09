//! DWM/ACCENT material effects; cache only successful native application.
use super::windows::{pod_bar, pod_panel};
use crate::settings::Pod;
use crate::state::AppState;
use crate::{events, win};
use std::{sync::atomic::Ordering, time::Duration};
use tauri::{AppHandle, Manager, PhysicalSize, WebviewWindow};
/// 清除 DWM systembackdrop（tauri set_effects）。亚克力改走随窗口常驻的
/// ACCENT 通道，这里只负责切换材质前清掉旧 systembackdrop，避免旧版
/// 升级残留的 backdrop 与 ACCENT 模糊叠加。
pub(super) fn clear_system_backdrop(window: &WebviewWindow) -> bool {
    window.set_effects(None).is_ok()
}

/// 轻推窗口尺寸（-1 物理像素、跨一帧后还原）强制 DWM / WebView2 重新合成。
///
/// ACCENT 材质写入与 RedrawWindow 只作用于 GDI 表面；WebView2 的
/// DirectComposition 表面在窗口几何变化时才重新呈现，否则应用材质后
/// 画面停留在旧合成结果（表现为「材质应用了但没刷新出来」）。
/// 抖动等价于用户手动 resize 触发的重合成，中间态只存在约一帧，
/// 视觉上不可感知。
pub(super) fn nudge_recomposite(window: &WebviewWindow) {
    let Ok(size) = window.inner_size() else {
        return;
    };
    let width = size.width.max(2);
    let height = size.height.max(2);
    let _ = window.set_size(PhysicalSize::new(width - 1, height));
    // 两次 set_size 必须跨合成帧：DWM 每帧只采样一次窗口几何，同帧内连续
    // 两次变化会被合并成「净变化为零」，WebView2 的 DirectComposition 表面
    // 不会重新呈现，材质写入后画面仍停留在旧合成结果。等待约一帧让中间
    // 尺寸先被合成，还原尺寸时才有真实的几何变化触发重合成。
    std::thread::sleep(Duration::from_millis(16));
    let _ = window.set_size(PhysicalSize::new(width, height));
}

/// 透明浮层材质落地：供匣浮动面板与右键菜单共用，按材质分流到不同系统机制。
///
/// 亚克力走 SWCA ACCENT（见 win::apply_panel_acrylic）：DWM 系统背景
/// （tauri set_effects 的 systembackdrop）在窗口失焦后移除整个 backdrop
/// 且重放无效，而浮动面板免激活显示、绝大多数时间不持有焦点，是
/// 「不聚焦就看不到材质」的根源；ACCENT 策略随窗口常驻，聚焦与失焦
/// 表现一致。普通即无系统材质（云母已于 1.4.0 移除）。
/// 任何分支都先清掉 ACCENT 与 DWM effect 两个通道，避免材质切换时叠加。
pub(crate) fn apply_window_material(window: &WebviewWindow, material: &str) -> bool {
    let Ok(hwnd) = window.hwnd() else {
        return false;
    };
    win::disable_accent(hwnd.0 as isize);
    let _ = clear_system_backdrop(window);
    let applied = match material {
        "acrylic" => win::apply_panel_acrylic(hwnd.0 as isize),
        _ => true,
    };
    // 材质写入后主动重绘 GDI 表面并轻推一次合成，旧画面不再停留到
    // 下一次自然重绘。
    win::redraw_window(hwnd.0 as isize);
    nudge_recomposite(window);
    applied
}

/// 强制刷新指定匣边缘浮动条的无边框状态。透明 WebView2 的幽灵标题栏可能由
/// 兄弟浮动面板或右键菜单获得焦点触发，因此不能只在边缘浮动条自身收到焦点事件时修复。
pub(crate) fn refresh_pod_bar_chrome(app: &AppHandle, id: u64) {
    if let Some(bar) = pod_bar(app, id) {
        if let Ok(hwnd) = bar.hwnd() {
            win::prepare_shaped_window(hwnd.0 as isize);
        }
    }
}

/// 焦点变化时幂等重放浮动面板材质：重发一次全量材质，
/// 保证无论浮动面板是否持有焦点，材质属性始终处于已下发状态。
/// 边缘浮动条固定普通材质，无材质可重放，仅在显示 / 首摆路径做样式
/// 清理（见 prepare_shaped_window）；浮动面板额外重放框架抑制
/// （只写 DWM 属性，不动样式位）。
pub fn refresh_window_material(app: &AppHandle, label: &str) {
    let target = match events::pod_window(label) {
        Some(events::PodWindow::Bar(id)) => pod_bar(app, id).map(|window| (id, window, true)),
        Some(events::PodWindow::Panel(id)) => pod_panel(app, id).map(|window| (id, window, false)),
        None => None,
    };
    let Some((id, window, is_bar)) = target else {
        return;
    };
    // 任一匣窗口的焦点变化都同步刷新其边缘浮动条。用户点击浮动面板时边缘浮动条通常
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
    // 云母已移除，材质只剩亚克力一种需要重放；普通无原生材质可重放。
    if let Some("acrylic") = material.as_deref() {
        if let Ok(hwnd) = window.hwnd() {
            let _ = win::apply_panel_acrylic(hwnd.0 as isize);
            // 重放同样轻推一次合成，焦点切换后材质立即可见。
            nudge_recomposite(&window);
        }
    }
    // 浮动面板焦点变化时幂等重放一次框架抑制（只写两个 DWM 属性，
    // 不触碰样式位与框架——面板的系统阴影依赖它们）。
    if let Ok(hwnd) = window.hwnd() {
        win::suppress_panel_frame(hwnd.0 as isize);
    }
}

/// 浮动面板材质只在变化时重设（每次显示都重设亚克力会引起重绘闪烁）。
///
/// 必须对隐藏 / 未固定的浮动面板同样生效：此前只在浮动面板可见时应用，而运行态
/// 已经记录了新材质，显示路径的变化检测就再也不触发，导致「改材质时
/// 浮动面板没固定」永远不生效。
pub(super) fn apply_panel_material_if_changed(app: &AppHandle, pod: &Pod) {
    let material = if app
        .state::<AppState>()
        .accessibility_reduce_transparency
        .load(Ordering::Relaxed)
    {
        "plain"
    } else {
        pod.panel_material.as_str()
    };
    let changed = {
        let state = app.state::<AppState>();
        let mut guard = state.pods.lock().unwrap();
        let runtime = guard.entry(pod.id).or_default();
        runtime.panel_material.as_deref() != Some(material)
    };
    if changed {
        if let Some(panel) = pod_panel(app, pod.id) {
            if apply_window_material(&panel, material) {
                let state = app.state::<AppState>();
                let mut guard = state.pods.lock().unwrap();
                if let Some(runtime) = guard.get_mut(&pod.id) {
                    runtime.panel_material = Some(material.to_string());
                }
            }
        }
        // 材质策略变化会触发 DWM 重新合成同一显示器上的兄弟窗口；
        // 顺带刷新边缘浮动条渲染，保证所有匣窗口与新材料状态一致。
        refresh_pod_bar_chrome(app, pod.id);
    }
}
