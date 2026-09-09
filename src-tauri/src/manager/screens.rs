use crate::settings::Pod;
use serde::Serialize;
use tauri::AppHandle;
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct MonitorGeometry {
    pub(super) rect: (i32, i32, i32, i32),
    pub(super) scale_factor: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq, schemars::JsonSchema)]
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

/// 找到匣所在显示器：按名称匹配，空名或未找到回退主显示器。
///
/// Tauri 的显示器位置/尺寸是物理像素；同时返回目标显示器的缩放率，避免
/// 浮动面板窗口尚在旧显示器时误用 `panel.scale_factor()` 计算新位置和尺寸。
pub(super) fn monitor(app: &AppHandle, pod: &Pod) -> Option<MonitorGeometry> {
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
