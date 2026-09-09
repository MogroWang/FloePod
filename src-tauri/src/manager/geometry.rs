//! Pure physical-pixel geometry; monitor discovery and native effects are separate.
const POD_BAR_ACCEPT_GROW: u32 = 18;
const PANEL_GAP: u32 = 10;
const PANEL_MARGIN: u32 = 8;
/// 边缘浮动条窗口的几何（长边方向由边缘决定）。
/// 短边来自匣的 bar_width 设置，长边来自 bar_length 设置。
pub(super) fn bar_geometry_for_monitor(
    monitor: (i32, i32, i32, i32),
    edge: &str,
    offset: f64,
    accepting: bool,
    scale: f64,
    bar_width: u32,
    bar_length: u32,
) -> (i32, i32, i32, i32) {
    let (mx, my, mw, mh) = monitor;
    let short_value = if accepting {
        bar_width.saturating_add(POD_BAR_ACCEPT_GROW)
    } else {
        bar_width
    };
    let short = scale_logical_px(short_value, scale);
    let long = scale_logical_px(bar_length, scale);
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

/// 把请求的浮动面板矩形限制在显示器工作矩形内。
///
/// 先限制宽高再 clamp 坐标，保证在高 DPI、窄屏或异常尺寸下也不会出现
/// `min > max` 导致 Rust `clamp` panic。
pub(super) fn scale_logical_px(value: u32, scale: f64) -> i32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (value as f64 * scale).round().clamp(1.0, i32::MAX as f64) as i32
}

pub(super) fn panel_geometry(
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
