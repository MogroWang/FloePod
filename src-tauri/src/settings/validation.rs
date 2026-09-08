use super::model::{valid_material, Settings};
use crate::file_paths::{path_is_within, paths_equal, resolve_config_path, resolve_path};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 校验十六进制颜色：空串（跟随主题）或 #RGB / #RRGGBB / #RRGGBBAA。
fn valid_hex_color(raw: &str) -> bool {
    let body = match raw.strip_prefix('#') {
        Some(body) => body,
        None => return false,
    };
    if !matches!(body.len(), 3 | 6 | 8) {
        return false;
    }
    body.chars().all(|c| c.is_ascii_hexdigit())
}

/// 完整验证将要持久化或用于文件操作的设置。
pub fn validate(s: &Settings, data_dir: &str) -> Result<(), String> {
    validate_impl(s, data_dir, true)
}

/// 文件操作只验证实际涉及的 pod，并要求它的磁盘/共享根当前可访问。
/// 这样一个离线的无关移动盘不会阻断其他 pod，同时也不会把“盘符离线”误判为条目已删除。
pub fn validate_pod_for_io(s: &Settings, data_dir: &str, pod_id: u64) -> Result<(), String> {
    let pod = s
        .pods
        .iter()
        .find(|pod| pod.id == pod_id)
        .cloned()
        .ok_or_else(|| format!("匣不存在: {pod_id}"))?;
    let mut isolated = s.clone();
    isolated.pods = vec![pod];
    validate_impl(&isolated, data_dir, false)
}

fn validate_impl(s: &Settings, data_dir: &str, allow_missing_roots: bool) -> Result<(), String> {
    if !matches!(s.theme.as_str(), "system" | "light" | "dark") {
        return Err(format!("未知主题: {}", s.theme));
    }

    if s.auto_block.apps.len() > 64 {
        return Err("自动屏蔽应用列表过长".into());
    }
    if !s.accessibility.scale.is_finite() || !(1.0..=2.0).contains(&s.accessibility.scale) {
        return Err("辅助功能缩放比例必须在 100% 到 200% 之间".into());
    }
    for app in &s.auto_block.apps {
        if app.trim().trim_matches('"').is_empty() {
            return Err("自动屏蔽应用名不能为空".into());
        }
        if app.chars().count() > 260 {
            return Err("自动屏蔽应用名过长".into());
        }
    }

    let data_dir = resolve_path(Path::new(data_dir))?;
    let mut ids = HashSet::new();
    let mut folders: Vec<(u64, String, PathBuf)> = Vec::new();

    for pod in &s.pods {
        if pod.id == 0 || !ids.insert(pod.id) {
            return Err(format!("匣 ID 无效或重复: {}", pod.id));
        }
        if pod.name.trim().is_empty() {
            return Err(format!("匣 {} 的名称不能为空", pod.id));
        }
        if !matches!(pod.edge.as_str(), "top" | "right" | "bottom" | "left") {
            return Err(format!("匣「{}」的屏幕边缘无效", pod.name));
        }
        if !pod.offset.is_finite() || !(0.0..=1.0).contains(&pod.offset) {
            return Err(format!("匣「{}」的位置无效", pod.name));
        }
        if !pod.opacity.is_finite() || !(0.1..=1.0).contains(&pod.opacity) {
            return Err(format!("匣「{}」的不透明度无效", pod.name));
        }
        if !valid_material(&pod.material) {
            return Err(format!("匣「{}」的材质无效", pod.name));
        }
        if !valid_material(&pod.panel_material) {
            return Err(format!("匣「{}」的浮动面板材质无效", pod.name));
        }
        if pod.rules.allowed_extensions.len() > 64
            || pod
                .rules
                .allowed_extensions
                .iter()
                .any(|extension| extension.len() > 32 || extension.contains(['/', '\\']))
        {
            return Err(format!("匣「{}」的扩展名规则无效", pod.name));
        }
        if pod.rules.name_contains.chars().count() > 128 {
            return Err(format!("匣「{}」的文件名规则过长", pod.name));
        }
        if pod.rules.max_size_mb > 102_400 {
            return Err(format!("匣「{}」的文件大小规则超过 100GB", pod.name));
        }
        if !matches!(pod.rules.duplicate_policy.as_str(), "allow" | "reject") {
            return Err(format!("匣「{}」的重复文件规则无效", pod.name));
        }
        if pod.rules.expire_days > 3_650 {
            return Err(format!("匣「{}」的到期天数不能超过 10 年", pod.name));
        }
        if pod.security.auto_lock_minutes > 24 * 60 {
            return Err(format!("匣「{}」的自动锁定时间不能超过 24 小时", pod.name));
        }
        if pod.security.retention_days > 3_650 {
            return Err(format!("匣「{}」的保留期限不能超过 10 年", pod.name));
        }
        for pattern in [&pod.rules.rename_pattern, &pod.rules.subfolder_pattern] {
            if pattern.chars().count() > 180 || pattern.contains("..") {
                return Err(format!("匣「{}」的规则路径模板无效", pod.name));
            }
        }
        if !pod.panel_opacity.is_finite() || !(0.1..=1.0).contains(&pod.panel_opacity) {
            return Err(format!("匣「{}」的浮动面板不透明度无效", pod.name));
        }
        if pod.auto_hide_delay_ms > 5000 {
            return Err(format!("匣「{}」的自动隐藏延迟无效", pod.name));
        }
        if pod.stealth_delay_ms > 60_000 {
            return Err(format!("匣「{}」的隐匿延迟无效", pod.name));
        }
        if !(410..=600).contains(&pod.panel_width) {
            return Err(format!("匣「{}」的浮动面板宽度无效", pod.name));
        }
        if pod.hover_delay_ms > 600 {
            return Err(format!("匣「{}」的悬停延迟无效", pod.name));
        }
        if !matches!(
            pod.drop_action.as_str(),
            "ask" | "copy" | "move" | "shortcut"
        ) {
            return Err(format!("匣「{}」的拖入动作无效", pod.name));
        }
        if !(28..=96).contains(&pod.bar_width) {
            return Err(format!("匣「{}」的浮动条宽度无效", pod.name));
        }
        if !(100..=500).contains(&pod.bar_length) {
            return Err(format!("匣「{}」的浮动条长度无效", pod.name));
        }
        if !pod.bar_color.is_empty() && !valid_hex_color(&pod.bar_color) {
            return Err(format!("匣「{}」的浮动条填充色无效", pod.name));
        }
        if pod.corner_radius > 64 {
            return Err(format!("匣「{}」的圆角无效", pod.name));
        }
        if !pod.panel_color.is_empty() && !valid_hex_color(&pod.panel_color) {
            return Err(format!("匣「{}」的浮动面板填充色无效", pod.name));
        }
        if !pod.border_color.is_empty() && !valid_hex_color(&pod.border_color) {
            return Err(format!("匣「{}」的边框颜色无效", pod.name));
        }
        if !pod.border_opacity.is_finite() || !(0.0..=1.0).contains(&pod.border_opacity) {
            return Err(format!("匣「{}」的边框不透明度无效", pod.name));
        }

        let raw = pod.staging_folder.trim();
        if raw.is_empty() {
            if pod.enabled {
                return Err(format!("匣「{}」尚未选择暂存文件夹", pod.name));
            }
            // 兼容旧版或未配置完成的禁用匣；重新启用前仍必须选择安全目录。
            continue;
        }
        let folder = if allow_missing_roots {
            resolve_config_path(Path::new(raw))?
        } else {
            resolve_path(Path::new(raw))?
        };
        if folder.parent().is_none() {
            return Err(format!(
                "不能把磁盘或共享根目录设为暂存文件夹: {}",
                folder.display()
            ));
        }
        if path_is_within(&folder, &data_dir) || path_is_within(&data_dir, &folder) {
            return Err("暂存文件夹不能与 FloePod 数据目录相同或互相包含".into());
        }
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            if let Ok(profile) = resolve_path(Path::new(&profile)) {
                if paths_equal(&folder, &profile) || path_is_within(&profile, &folder) {
                    return Err("不能把整个用户目录或其父目录设为暂存文件夹".into());
                }
            }
        }
        for key in ["WINDIR", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(protected) = std::env::var_os(key) {
                if let Ok(protected) = resolve_path(Path::new(&protected)) {
                    if path_is_within(&folder, &protected) {
                        return Err(format!(
                            "不能把系统目录设为暂存文件夹: {}",
                            folder.display()
                        ));
                    }
                }
            }
        }
        folders.push((pod.id, pod.name.clone(), folder));
    }

    for i in 0..folders.len() {
        for j in (i + 1)..folders.len() {
            let (_, name_a, a) = &folders[i];
            let (_, name_b, b) = &folders[j];
            if path_is_within(a, b) || path_is_within(b, a) {
                return Err(format!(
                    "匣「{name_a}」与「{name_b}」的暂存文件夹不能相同或互相嵌套"
                ));
            }
        }
    }
    Ok(())
}
