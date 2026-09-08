use super::model::{Pod, Settings};

/// 读取和保存补丁必须经过同一迁移链，且只修改内存候选值。
pub(super) fn decode(value: serde_json::Value) -> Result<Settings, String> {
    let mut settings: Settings =
        serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    migrate_legacy(&mut settings, &value);
    migrate_panel_appearance(&mut settings, &value);
    normalize_materials(&mut settings);
    normalize_panel_width(&mut settings);
    Ok(settings)
}

/// 旧版（0.2/0.3）单个暂存配置 -> 生成一个默认「匣」，保证老用户升级不丢配置。
fn migrate_legacy(s: &mut Settings, v: &serde_json::Value) {
    if !s.pods.is_empty() {
        return;
    }
    let folder = match v.get("stagingFolder").and_then(|x| x.as_str()) {
        Some(f) if !f.is_empty() => f.to_string(),
        _ => return,
    };
    let edge = v
        .get("edge")
        .and_then(|x| x.as_str())
        .filter(|e| matches!(*e, "top" | "right" | "bottom" | "left"))
        .unwrap_or("left");
    let material = v
        .get("material")
        .and_then(|x| x.as_str())
        .unwrap_or("acrylic")
        .to_string();
    let opacity = v.get("opacity").and_then(|x| x.as_f64()).unwrap_or(1.0);
    s.pods.push(Pod {
        id: 1,
        name: "我的匣".into(),
        edge: edge.into(),
        monitor: String::new(),
        offset: 0.5,
        staging_folder: folder,
        panel_opacity: opacity,
        material: material.clone(),
        panel_material: material,
        panel_width: v.get("panelWidth").and_then(|x| x.as_u64()).unwrap_or(440) as u32,
        opacity,
        hover_delay_ms: v
            .get("hoverDelayMs")
            .and_then(|x| x.as_u64())
            .unwrap_or(120),
        drop_action: v
            .get("dropAction")
            .and_then(|x| x.as_str())
            .unwrap_or("ask")
            .into(),
        enabled: true,
        ..Pod::default()
    });
}

/// 「模糊」（Win10 BlurBehind）与亚克力观感一致且在 Win11 上渲染异常，
/// 已从材质列表移除：存量配置里的 blur 统一迁移为 acrylic。
/// 边缘浮动条自 1.3.0 起不再提供材质设置（固定为普通半透明，见 Pod::material），
/// 存量配置里的边缘浮动条材质在此一并废弃。
/// 浮动面板的「模糊」与亚克力观感一致、云母随 1.4.0 移除：两者统一迁移为亚克力。
fn normalize_materials(s: &mut Settings) {
    for pod in &mut s.pods {
        pod.material = "plain".into();
        if pod.panel_material == "blur" || pod.panel_material == "mica" {
            pod.panel_material = "acrylic".into();
        }
    }
}

/// 1.5.0 起浮动面板宽度收紧为 410-600：旧配置的存量值收敛到新范围，
/// 否则严格校验会让 watcher 与保存路径全部报错。
fn normalize_panel_width(s: &mut Settings) {
    for pod in &mut s.pods {
        pod.panel_width = pod.panel_width.clamp(410, 600);
    }
}

/// 1.2 及更早的存储没有浮动面板独立外观字段：浮动面板沿用该匣的材质与不透明度。
/// 只回填存储中确实缺失的字段，避免每次加载覆盖用户已保存的值。
fn migrate_panel_appearance(s: &mut Settings, v: &serde_json::Value) {
    let Some(raw_pods) = v.get("pods").and_then(|p| p.as_array()) else {
        return;
    };
    for (index, raw) in raw_pods.iter().enumerate() {
        let Some(pod) = s.pods.get_mut(index) else {
            break;
        };
        if raw.get("panelMaterial").is_none() {
            pod.panel_material = pod.material.clone();
        }
        if raw.get("panelOpacity").is_none() {
            pod.panel_opacity = pod.opacity;
        }
    }
}
