//! Window registry, creation, destruction and placement. All native handles are resolved by label at use time.
use super::geometry::{bar_geometry_for_monitor, panel_geometry, scale_logical_px};
use super::pod_of;
use super::screens::monitor;
use crate::settings::{Pod, Settings};
use crate::state::AppState;
use crate::{events, win};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WebviewWindowBuilder,
};
pub fn pod_bar(app: &AppHandle, id: u64) -> Option<WebviewWindow> {
    app.get_webview_window(&events::pod_bar_label(id))
}

pub fn pod_panel(app: &AppHandle, id: u64) -> Option<WebviewWindow> {
    app.get_webview_window(&events::pod_panel_label(id))
}

/// 边缘浮动条窗口的几何与所在显示器缩放率（物理像素）。
pub(super) fn bar_geometry(
    app: &AppHandle,
    pod: &Pod,
    accepting: bool,
) -> Option<((i32, i32, i32, i32), f64)> {
    let target = monitor(app, pod)?;
    Some((
        bar_geometry_for_monitor(
            target.rect,
            &pod.edge,
            pod.offset,
            accepting,
            target.scale_factor,
            pod.bar_width,
            pod.bar_length,
        ),
        target.scale_factor,
    ))
}

pub fn place_pod_bar(app: &AppHandle, pod: &Pod, accepting: bool) {
    let Some(bar) = pod_bar(app, pod.id) else {
        return;
    };
    let Some(((x, y, w, h), scale)) = bar_geometry(app, pod, accepting) else {
        return;
    };
    if let Err(error) = bar
        .set_size(PhysicalSize::new(w as u32, h as u32))
        .and_then(|_| bar.set_position(PhysicalPosition::new(x, y)))
    {
        crate::logging::write(&format!(
            "[window] 浮动条摆放失败，将在下次摆放时重试: {error}"
        ));
        return;
    }
    // 记录最近一次摆放的几何与缩放率：隐匿模式的看门狗用它判断指针是否
    // 靠近，避免每次 tick 都读库。
    let region_stale = {
        let state = app.state::<AppState>();
        let mut guard = state.pods.lock().unwrap();
        let runtime = guard.entry(pod.id).or_default();
        runtime.bar_rect = Some((x, y, w, h));
        runtime.bar_scale = scale;
        // SetWindowRgn 会触发一次完整的框架重算并打断 WebView2 合成；
        // 拖动边缘浮动条（move_pod_bar）的每个指针事件都会走到这里，
        // 只有几何真正变化时才重新应用同形矩形区域（首摆、接纳态加宽、
        // 改宽度/长度设置、跨显示器缩放率变化等）。
        runtime.bar_region_size != Some((w, h))
    };
    if region_stale {
        if let Ok(hwnd) = bar.hwnd() {
            // 同形矩形区域裁掉 Windows 11 在窗口矩形外延伸的系统框架，
            // 顺带完成该窗口的首轮样式位清理（见 set_bar_region）。
            if win::set_bar_region(hwnd.0 as isize, w, h, 0, &pod.edge) {
                let state = app.state::<AppState>();
                let mut guard = state.pods.lock().unwrap();
                if let Some(runtime) = guard.get_mut(&pod.id) {
                    if runtime.bar_rect == Some((x, y, w, h)) {
                        runtime.bar_region_size = Some((w, h));
                    }
                }
            }
        }
    }
}

/// 浮动面板：贴着匣弹出，长边方向垂直/水平时对齐到匣中心。
pub(super) fn place_panel(app: &AppHandle, pod: &Pod) {
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
        pod.bar_length,
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

pub(super) fn ensure_pod_windows(app: &AppHandle, pod: &Pod) {
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
        // 边缘浮动条不设置标题文字：即使非客户区渲染被系统事件短暂恢复，
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
        .title(format!("{} 浮动面板", pod.name))
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
    // 边缘浮动条形状由前端自绘：禁用 Windows 11 系统窗口圆角，
    // 否则 DWM 圆角会把贴边的圆角矩形裁掉，看起来「显示不全」。
    if let Some(bar) = pod_bar(app, pod.id) {
        // SetWindowSubclass 必须在窗口所属线程调用；run_on_main_thread 与
        // 后续窗口操作按顺序分派，确保首次显示前安装。按标签重新取得窗口，
        // 避免禁用/重建匣后使用已销毁的 HWND。重复安装同一回调是幂等的。
        let handle = app.clone();
        let id = pod.id;
        if let Err(error) = bar.run_on_main_thread(move || {
            if let Some(bar) = pod_bar(&handle, id) {
                if let Ok(hwnd) = bar.hwnd() {
                    if !win::install_bar_chrome_guard(hwnd.0 as isize) {
                        crate::logging::write("[window] 安装浮动条无边框消息处理失败");
                    }
                }
            }
        }) {
            crate::logging::write(&format!("[window] 分派浮动条初始化失败: {error}"));
        }
        if let Ok(hwnd) = bar.hwnd() {
            win::disable_rounding(hwnd.0 as isize);
            // 材质需要用窗口区域（region）裁剪成胶囊形状：先清掉样式里残留的
            // 标题栏位并关闭 DWM 非客户区渲染，否则区域会让系统画出标题栏。
            win::prepare_shaped_window(hwnd.0 as isize);
        }
    }
    // 浮动面板：请求系统圆角，与 CSS 的 clip-path 圆角轮廓对齐；
    // 只压制 Win11 的 1px 外描边与焦点过渡（suppress_panel_frame），
    // 绝不动样式位与框架——面板的系统阴影依赖它们（见该函数注释）。
    if let Some(panel) = pod_panel(app, pod.id) {
        if let Ok(hwnd) = panel.hwnd() {
            win::prefer_rounded_corners(hwnd.0 as isize);
            win::suppress_panel_frame(hwnd.0 as isize);
        }
    }
    place_pod_bar(app, pod, false);
}

pub(super) fn destroy_pod_windows(app: &AppHandle, id: u64) {
    let labels = [events::pod_bar_label(id), events::pod_panel_label(id)];
    for l in labels {
        if let Some(w) = app.get_webview_window(&l) {
            let _ = w.destroy();
        }
    }
}

/// 让所有窗口与当前设置中的匣对齐：创建缺失、销毁多余的。
pub(super) fn sync_pods_with_settings(app: &AppHandle, s: &Settings) {
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

/// 拖动边缘浮动条过程中实时重定位（不写库）；松手后由 update_pod 持久化 offset。
pub fn move_pod_bar(app: &AppHandle, id: u64, offset: f64) {
    let Some(mut pod) = pod_of(app, id) else {
        return;
    };
    pod.offset = offset.clamp(0.0, 1.0);
    place_pod_bar(app, &pod, false);
}

pub fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
