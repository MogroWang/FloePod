use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db;

pub const KEY: &str = "app";
const NEXT_POD_ID_KEY: &str = "next_pod_id";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Hotkeys {
    #[serde(default = "d_toggle_bar")]
    pub toggle_bar: String,
    #[serde(default = "d_collect_clipboard")]
    pub collect_clipboard: String,
    #[serde(default = "d_open_panel")]
    pub open_panel: String,
    #[serde(default = "d_lock_sensitive")]
    pub lock_sensitive: String,
}

fn d_toggle_bar() -> String {
    "Alt+Shift+F".into()
}
fn d_collect_clipboard() -> String {
    "Alt+Shift+S".into()
}
fn d_open_panel() -> String {
    "Alt+Shift+P".into()
}
fn d_lock_sensitive() -> String {
    "Alt+Shift+L".into()
}

impl Hotkeys {
    pub fn with_defaults() -> Self {
        Self {
            toggle_bar: d_toggle_bar(),
            collect_clipboard: d_collect_clipboard(),
            open_panel: d_open_panel(),
            lock_sensitive: d_lock_sensitive(),
        }
    }
}

/// 自动屏蔽：配置的应用位于前台时暂时隐藏全部匣，离开前台后自动恢复。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutoBlock {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub apps: Vec<String>,
}

/// 免费的「辅助功能」设置：提高可读性、提供非拖拽替代并减少认知负担。
/// 各选项相互独立、直接生效；1.5.0 起不再有「启用辅助功能」总开关
/// （旧配置中的 enabled 字段被 serde 忽略）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Accessibility {
    /// WebView 内容缩放，范围 1.0 - 2.0。
    pub scale: f64,
    pub high_contrast: bool,
    pub reduce_transparency: bool,
    pub reduce_motion: bool,
    pub simple_language: bool,
    pub confirm_dangerous: bool,
    pub send_to_menu: bool,
}

/// 单个匣的本地规则。规则只做可解释的过滤、命名、归档和校验，不执行任意脚本。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PodRules {
    pub enabled: bool,
    pub template: String,
    pub allowed_extensions: Vec<String>,
    pub name_contains: String,
    pub source_folder: String,
    pub max_size_mb: u64,
    /// 支持 {name}、{stem}、{ext}、{date}、{year}、{month}、{day}。
    pub rename_pattern: String,
    /// 支持日期令牌；必须是相对目录且不能含 ..。
    pub subfolder_pattern: String,
    /// allow / reject
    pub duplicate_policy: String,
    pub checksum_sidecar: bool,
    pub expire_days: u32,
    pub remove_after_export: bool,
}

impl Default for PodRules {
    fn default() -> Self {
        Self {
            enabled: false,
            template: "manual".into(),
            allowed_extensions: Vec::new(),
            name_contains: String::new(),
            source_folder: String::new(),
            max_size_mb: 0,
            rename_pattern: "{name}".into(),
            subfolder_pattern: String::new(),
            duplicate_policy: "allow".into(),
            checksum_sidecar: false,
            expire_days: 0,
            remove_after_export: false,
        }
    }
}

/// 敏感匣使用 Windows EFS 加密目录，并以 Windows Hello 控制应用内解锁。
/// 不保存自制密码或密钥；不支持 EFS 的卷会拒绝启用而不是假装已加密。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PodSecurity {
    pub enabled: bool,
    pub require_windows_hello: bool,
    pub auto_lock_minutes: u32,
    pub retention_days: u32,
    pub cleanup_after_export: bool,
    pub suppress_thumbnails: bool,
    pub suppress_index: bool,
}

impl Default for PodSecurity {
    fn default() -> Self {
        Self {
            enabled: false,
            require_windows_hello: true,
            auto_lock_minutes: 10,
            retention_days: 0,
            cleanup_after_export: false,
            suppress_thumbnails: true,
            suppress_index: true,
        }
    }
}

impl Default for Accessibility {
    fn default() -> Self {
        Self {
            scale: 1.0,
            high_contrast: false,
            reduce_transparency: false,
            reduce_motion: false,
            simple_language: false,
            confirm_dangerous: true,
            send_to_menu: false,
        }
    }
}

/// 窗口材质取值:亚克力 / 普通无材质。
/// 早期版本的「模糊」与亚克力观感一致、云母因系统材质失焦不可靠已移除,
/// 存量配置统一迁移(见 normalize_materials)。
pub const MATERIALS: [&str; 2] = ["acrylic", "plain"];

fn valid_material(material: &str) -> bool {
    MATERIALS.contains(&material)
}

/// 一个「匣」：贴在屏幕边缘的独立暂存点，拥有自己的保存文件夹与外观。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Pod {
    pub id: u64,
    pub name: String,
    /// top / right / bottom / left
    pub edge: String,
    /// 显示器名；空串 = 主显示器
    pub monitor: String,
    /// 沿边缘的位置 0.0 - 1.0
    pub offset: f64,
    pub staging_folder: String,
    pub opacity: f64,
    /// 边缘浮动条材质；1.3.0 起废弃，normalize 时固定为 "plain"（普通半透明）。
    /// 保留字段仅为兼容旧存储与 IPC 结构，应用层不再读取。
    pub material: String,
    /// 浮动面板材质；与边缘浮动条材质独立设置。
    pub panel_material: String,
    /// 浮动面板不透明度 0.1 - 1.0；与边缘浮动条不透明度独立设置。
    pub panel_opacity: f64,
    /// 浮动面板填充色（#RGB/#RRGGBB/#RRGGBBAA）；空串 = 跟随主题表面色。
    pub panel_color: String,
    pub panel_width: u32,
    pub hover_delay_ms: u64,
    /// 是否允许悬停自动弹出；关闭后仍可单击或用键盘打开浮动面板。
    pub hover_open: bool,
    /// 鼠标离开后自动隐藏浮动面板（淡出；重新悬停时淡入）。
    pub auto_hide: bool,
    /// 鼠标离开后到自动隐藏的延迟（毫秒）。
    pub auto_hide_delay_ms: u64,
    /// 隐匿模式：无交互超过延迟后边缘浮动条淡化隐去，指针靠近时再淡入。
    pub stealth: bool,
    /// 隐匿模式下无交互到淡化隐去的延迟（毫秒）。
    pub stealth_delay_ms: u64,
    pub drop_action: String,
    pub enabled: bool,
    /// 边缘浮动条短边宽度（CSS 逻辑像素）；浮动面板宽度由 panel_width 控制。
    pub bar_width: u32,
    /// 边缘浮动条长度，即沿屏幕边缘方向的长边（CSS 逻辑像素）。
    pub bar_length: u32,
    /// 边缘浮动条填充色（#RGB/#RRGGBB/#RRGGBBAA）；空串 = 跟随主题表面色。
    pub bar_color: String,
    /// 边缘浮动条外角圆角半径；0 为直角，CSS 会自动把超过半宽的值收敛。
    pub corner_radius: u32,
    /// 边缘浮动条边框颜色（#RGB/#RRGGBB/#RRGGBBAA）；空串 = 跟随主题。
    pub border_color: String,
    /// 边缘浮动条边框不透明度 0.0 - 1.0，作用于 border_color 或主题默认边框色。
    pub border_opacity: f64,
    #[serde(default)]
    pub rules: PodRules,
    #[serde(default)]
    pub security: PodSecurity,
}

impl Default for Pod {
    fn default() -> Self {
        Pod {
            id: 0,
            name: "新匣".into(),
            edge: "left".into(),
            monitor: String::new(),
            offset: 0.5,
            staging_folder: String::new(),
            opacity: 1.0,
            material: "plain".into(),
            panel_material: "acrylic".into(),
            panel_opacity: 1.0,
            panel_color: String::new(),
            panel_width: 440,
            hover_delay_ms: 120,
            hover_open: true,
            auto_hide: true,
            auto_hide_delay_ms: 320,
            stealth: false,
            stealth_delay_ms: 3000,
            drop_action: "ask".into(),
            enabled: true,
            bar_width: 44,
            bar_length: 190,
            bar_color: String::new(),
            corner_radius: 22,
            border_color: String::new(),
            border_opacity: 1.0,
            rules: PodRules::default(),
            security: PodSecurity::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "d_theme")]
    pub theme: String,
    #[serde(default)]
    pub first_run_done: bool,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default = "Hotkeys::with_defaults")]
    pub hotkeys: Hotkeys,
    #[serde(default)]
    pub auto_block: AutoBlock,
    #[serde(default)]
    pub accessibility: Accessibility,
    #[serde(default)]
    pub pods: Vec<Pod>,
    /// 只读：由应用在读取时注入并返回前端，但不接受数据库中的旧值。
    /// `persist` 会在写库前显式剔除这两个运行时字段。
    #[serde(skip_deserializing, default)]
    pub version: String,
    #[serde(skip_deserializing, default)]
    pub data_dir: String,
}

fn d_theme() -> String {
    "system".into()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: d_theme(),
            first_run_done: false,
            autostart: false,
            hotkeys: Hotkeys::with_defaults(),
            auto_block: AutoBlock::default(),
            accessibility: Accessibility::default(),
            pods: Vec::new(),
            version: String::new(),
            data_dir: String::new(),
        }
    }
}

pub fn load(conn: &Connection, data_dir: &str, version: &str) -> Result<Settings, String> {
    let mut s: Settings = match db::kv_get(conn, KEY)? {
        Some(json) => {
            let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
            let mut s: Settings = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
            migrate_legacy(&mut s, &v);
            migrate_panel_appearance(&mut s, &v);
            normalize_materials(&mut s);
            normalize_panel_width(&mut s);
            s
        }
        None => Settings::default(),
    };
    crate::policy::apply_to_settings(&mut s)?;
    s.version = version.to_string();
    s.data_dir = data_dir.to_string();
    Ok(s)
}

/// 将绝对路径做词法归一化，并尽可能解析已存在祖先中的符号链接 / junction。
///
/// 暂存目录允许尚不存在，因此不能简单要求 `canonicalize()` 整条路径成功。
pub fn resolve_path(path: &Path) -> Result<PathBuf, String> {
    resolve_path_impl(path, false)
}

/// 配置校验允许可移动盘暂时离线；这种情况下只能保留词法归一化结果。
/// 真正读写文件时仍必须使用 [`resolve_path`]，从而把“盘符不可用”与“叶子不存在”区分开。
fn resolve_config_path(path: &Path) -> Result<PathBuf, String> {
    resolve_path_impl(path, true)
}

/// 比较两条持久化路径时不要求磁盘或共享在线；实际文件操作仍须先调用 [`resolve_path`]。
pub fn configured_paths_equal(a: &Path, b: &Path) -> Result<bool, String> {
    Ok(paths_equal(
        &resolve_config_path(a)?,
        &resolve_config_path(b)?,
    ))
}

fn resolve_path_impl(path: &Path, allow_missing_root: bool) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("路径必须是绝对路径: {}", path.display()));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(format!("路径越过根目录: {}", path.display()));
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    let mut cursor = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(cursor) {
            Ok(_) => {
                let mut resolved = cursor
                    .canonicalize()
                    .map_err(|e| format!("无法解析路径 {}: {e}", cursor.display()))?;
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = cursor.file_name() else {
                    return if allow_missing_root {
                        Ok(normalized)
                    } else {
                        Err(format!("路径所在磁盘或共享位置不可用: {}", path.display()))
                    };
                };
                missing.push(name.to_os_string());
                cursor = cursor
                    .parent()
                    .ok_or_else(|| format!("无法解析路径: {}", path.display()))?;
            }
            Err(e) => return Err(format!("无法访问路径 {}: {e}", cursor.display())),
        }
    }
}

fn component_eq(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// `path` 是否等于 `root` 或位于其下。调用方应先用 [`resolve_path`] 归一化。
pub fn path_is_within(path: &Path, root: &Path) -> bool {
    let path_parts: Vec<_> = path.components().map(|c| c.as_os_str()).collect();
    let root_parts: Vec<_> = root.components().map(|c| c.as_os_str()).collect();
    root_parts.len() <= path_parts.len()
        && root_parts
            .iter()
            .zip(path_parts.iter())
            .all(|(a, b)| component_eq(a, b))
}

pub fn paths_equal(a: &Path, b: &Path) -> bool {
    let a_parts: Vec<_> = a.components().map(|c| c.as_os_str()).collect();
    let b_parts: Vec<_> = b.components().map(|c| c.as_os_str()).collect();
    a_parts.len() == b_parts.len()
        && a_parts
            .iter()
            .zip(b_parts.iter())
            .all(|(x, y)| component_eq(x, y))
}

pub fn path_key(path: &Path) -> String {
    #[cfg(windows)]
    {
        path.to_string_lossy().replace('/', "\\").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().to_string()
    }
}

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

pub fn persist(conn: &Connection, s: &Settings) -> Result<(), String> {
    // version / dataDir 需要出现在 IPC 响应中供设置页展示，但它们是当前
    // 可执行文件与运行环境的派生值，不能写回数据库成为下次启动的输入。
    let mut value = serde_json::to_value(s).map_err(|e| e.to_string())?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "设置序列化结果必须是对象".to_string())?;
    object.remove("version");
    object.remove("dataDir");
    let json = serde_json::to_string(&value).map_err(|e| e.to_string())?;
    db::kv_set(conn, KEY, &json)
}

/// 用 patch 合并当前设置并持久化；返回合并后的完整设置。
/// pod 列表不通过此命令修改（走独立 pod 命令），仅合并标量字段。
pub fn merge_persist(
    conn: &Connection,
    patch: serde_json::Value,
    data_dir: &str,
    version: &str,
) -> Result<Settings, String> {
    let mut stored: serde_json::Map<String, serde_json::Value> = match db::kv_get(conn, KEY)? {
        Some(json) => serde_json::from_str(&json).map_err(|e| e.to_string())?,
        None => serde_json::Map::new(),
    };
    let obj = patch
        .as_object()
        .ok_or_else(|| "设置补丁必须是对象".to_string())?;
    for (k, v) in obj {
        match k.as_str() {
            "theme" | "firstRunDone" | "autostart" | "hotkeys" | "autoBlock" | "accessibility" => {
                stored.insert(k.clone(), v.clone());
            }
            "version" | "dataDir" | "pods" => {}
            _ => return Err(format!("未知设置字段: {k}")),
        }
    }

    // 必须先完整反序列化和验证，确认候选设置有效后才能覆盖数据库。
    let raw = serde_json::Value::Object(stored);
    let mut candidate: Settings = serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
    migrate_legacy(&mut candidate, &raw);
    migrate_panel_appearance(&mut candidate, &raw);
    normalize_materials(&mut candidate);
    candidate.version = version.to_string();
    candidate.data_dir = data_dir.to_string();
    validate(&candidate, data_dir)?;
    persist(conn, &candidate)?;
    Ok(candidate)
}

/// 在已加载的设置基础上分配下一个匣 ID 并持久化计数器（避免重复读库）。
pub fn next_pod_id_from(conn: &Connection, current: &Settings) -> Result<u64, String> {
    let floor = current
        .pods
        .iter()
        .map(|p| p.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| "匣 ID 已耗尽".to_string())?;
    let next = db::kv_get(conn, NEXT_POD_ID_KEY)?
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(floor)
        .max(floor);
    let following = next
        .checked_add(1)
        .ok_or_else(|| "匣 ID 已耗尽".to_string())?;
    db::kv_set(conn, NEXT_POD_ID_KEY, &following.to_string())?;
    Ok(next)
}

/// 在已加载的设置上插入 / 更新匣，验证后持久化。返回后 `current` 即持久化状态。
pub fn upsert_pod_from(
    conn: &Connection,
    current: &mut Settings,
    pod: &Pod,
    data_dir: &str,
) -> Result<(), String> {
    if let Some(existing) = current.pods.iter_mut().find(|p| p.id == pod.id) {
        *existing = pod.clone();
    } else {
        current.pods.push(pod.clone());
    }
    validate(current, data_dir)?;
    persist(conn, current)
}

pub fn delete_pod(
    conn: &Connection,
    id: u64,
    data_dir: &str,
    version: &str,
) -> Result<Settings, String> {
    let mut s = load(conn, data_dir, version)?;
    s.pods.retain(|p| p.id != id);
    persist(conn, &s)?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        db::migrate(&c).unwrap();
        c
    }

    fn pod(id: u64, folder: &Path) -> Pod {
        Pod {
            id,
            name: format!("匣 {id}"),
            edge: "left".into(),
            monitor: String::new(),
            offset: 0.5,
            staging_folder: folder.to_string_lossy().to_string(),
            opacity: 1.0,
            material: "acrylic".into(),
            panel_width: 440,
            hover_delay_ms: 120,
            drop_action: "ask".into(),
            enabled: true,
            ..Pod::default()
        }
    }

    #[test]
    fn legacy_settings_migrates_to_pod() {
        let c = conn();
        db::kv_set(
            &c,
            KEY,
            r#"{"stagingFolder":"D:\\暂存","edge":"right","firstRunDone":true}"#,
        )
        .unwrap();
        let s = load(&c, "DATA", "0.4.0").unwrap();
        assert!(s.first_run_done);
        assert_eq!(s.pods.len(), 1);
        assert_eq!(s.pods[0].staging_folder, "D:\\暂存");
        assert_eq!(s.pods[0].edge, "right");
        assert_eq!(s.pods[0].id, 1);
    }

    #[test]
    fn merge_ignores_pods_and_version() {
        let c = conn();
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data").to_string_lossy().to_string();
        db::kv_set(&c, KEY, r#"{"theme":"system","pods":[]}"#).unwrap();
        let s = merge_persist(
            &c,
            serde_json::json!({"theme":"dark","pods":[{"id":99}],"version":"9.9"}),
            &data_dir,
            "0.4.0",
        )
        .unwrap();
        assert_eq!(s.theme, "dark");
        assert!(s.pods.is_empty());
        assert_eq!(s.version, "0.4.0");
    }

    #[test]
    fn runtime_metadata_is_serialized_for_ipc_but_not_persisted() {
        let c = conn();
        let s = Settings {
            version: "1.0.0".into(),
            data_dir: r"C:\Users\tester\AppData\Roaming\FloePod".into(),
            ..Settings::default()
        };

        let wire = serde_json::to_value(&s).unwrap();
        assert_eq!(wire["version"], "1.0.0");
        assert_eq!(wire["dataDir"], s.data_dir);

        persist(&c, &s).unwrap();
        let stored: serde_json::Value =
            serde_json::from_str(&db::kv_get(&c, KEY).unwrap().unwrap()).unwrap();
        assert!(stored.get("version").is_none());
        assert!(stored.get("dataDir").is_none());
    }

    #[test]
    fn existing_v040_settings_round_trip_without_losing_fields() {
        let c = conn();
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data").to_string_lossy().to_string();
        let stage_one = tmp.path().join("stage-one").to_string_lossy().to_string();
        let stage_two = tmp.path().join("stage-two").to_string_lossy().to_string();
        let fixture = serde_json::json!({
            "theme": "dark",
            "firstRunDone": true,
            "autostart": true,
            "hotkeys": {
                "toggleBar": "Ctrl+Alt+KeyF",
                "collectClipboard": "Ctrl+Alt+KeyS",
                "openPanel": "Ctrl+Alt+KeyP"
            },
            "autoBlock": {
                "enabled": true,
                "apps": ["game.exe", "C:\\Games\\Racer.exe"]
            },
            "pods": [
                {
                    "id": 7,
                    "name": "左侧工作匣",
                    "edge": "left",
                    "monitor": "DISPLAY-A",
                    "offset": 0.25,
                    "stagingFolder": stage_one,
                    "opacity": 0.72,
                    "material": "plain",
                    "panelMaterial": "acrylic",
                    "panelOpacity": 0.9,
                    "panelColor": "#aabbcc",
                    "panelWidth": 512,
                    "hoverDelayMs": 600,
                    "autoHide": false,
                    "autoHideDelayMs": 480,
                    "stealth": true,
                    "stealthDelayMs": 5000,
                    "dropAction": "move",
                    "enabled": true,
                    "barWidth": 56,
                    "cornerRadius": 12,
                    "borderColor": "#80ffaa",
                    "borderOpacity": 0.4
                },
                {
                    "id": 19,
                    "name": "离线便携匣",
                    "edge": "bottom",
                    "monitor": "",
                    "offset": 1.0,
                    "stagingFolder": stage_two,
                    "opacity": 1.0,
                    "material": "plain",
                    "panelMaterial": "acrylic",
                    "panelOpacity": 1.0,
                    "panelColor": "",
                    "panelWidth": 300,
                    "hoverDelayMs": 0,
                    "autoHide": true,
                    "autoHideDelayMs": 320,
                    "stealth": false,
                    "stealthDelayMs": 3000,
                    "dropAction": "shortcut",
                    "enabled": false,
                    "barWidth": 36,
                    "cornerRadius": 0,
                    "borderColor": "",
                    "borderOpacity": 1.0
                }
            ]
        });
        db::kv_set(&c, KEY, &fixture.to_string()).unwrap();

        let loaded = load(&c, &data_dir, "1.0.0").unwrap();
        assert_eq!(loaded.pods.len(), 2);
        assert_eq!(loaded.pods[0].id, 7);
        assert_eq!(loaded.pods[0].monitor, "DISPLAY-A");
        assert_eq!(loaded.pods[0].panel_width, 512);
        assert_eq!(loaded.pods[0].bar_width, 56);
        assert_eq!(loaded.pods[0].corner_radius, 12);
        assert_eq!(loaded.pods[0].border_color, "#80ffaa");
        assert_eq!(loaded.pods[0].border_opacity, 0.4);
        assert_eq!(loaded.pods[0].panel_material, "acrylic");
        assert_eq!(loaded.pods[0].panel_opacity, 0.9);
        assert_eq!(loaded.pods[0].panel_color, "#aabbcc");
        assert!(!loaded.pods[0].auto_hide);
        assert_eq!(loaded.pods[0].auto_hide_delay_ms, 480);
        assert!(loaded.pods[0].stealth);
        assert_eq!(loaded.pods[0].stealth_delay_ms, 5000);
        assert_eq!(loaded.pods[1].drop_action, "shortcut");
        assert_eq!(loaded.pods[1].panel_material, "acrylic");
        assert!(loaded.pods[1].auto_hide);
        assert_eq!(loaded.pods[1].bar_width, 36);
        assert!(!loaded.pods[1].enabled);
        assert!(loaded.auto_block.enabled);
        assert_eq!(
            loaded.auto_block.apps,
            vec!["game.exe".to_string(), "C:\\Games\\Racer.exe".to_string()]
        );

        persist(&c, &loaded).unwrap();
        let stored: serde_json::Value =
            serde_json::from_str(&db::kv_get(&c, KEY).unwrap().unwrap()).unwrap();
        assert_eq!(stored["accessibility"]["enabled"], false);
        assert_eq!(stored["pods"][0]["hoverOpen"], true);
        assert_eq!(stored["pods"][0]["rules"]["enabled"], false);
        let mut legacy_view = stored;
        legacy_view.as_object_mut().unwrap().remove("accessibility");
        legacy_view["hotkeys"]
            .as_object_mut()
            .unwrap()
            .remove("lockSensitive");
        for pod in legacy_view["pods"].as_array_mut().unwrap() {
            pod.as_object_mut().unwrap().remove("hoverOpen");
            pod.as_object_mut().unwrap().remove("rules");
            pod.as_object_mut().unwrap().remove("security");
        }
        assert_eq!(legacy_view, fixture);
    }

    #[test]
    fn panel_appearance_migrates_from_bar_fields() {
        let c = conn();
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data").to_string_lossy().to_string();
        let stage = tmp.path().join("stage").to_string_lossy().to_string();
        // 1.2 版存储：pods 里没有 panelMaterial / panelOpacity。
        db::kv_set(
            &c,
            KEY,
            &serde_json::json!({
                "theme": "system",
                "pods": [{
                    "id": 1, "name": "匣", "edge": "left",
                    "stagingFolder": stage, "opacity": 0.8, "material": "acrylic"
                }]
            })
            .to_string(),
        )
        .unwrap();
        let s = load(&c, &data_dir, "1.3.0").unwrap();
        assert_eq!(s.pods[0].panel_material, "acrylic");
        assert!((s.pods[0].panel_opacity - 0.8).abs() < 1e-9);
        assert!(s.pods[0].auto_hide);
        assert_eq!(s.pods[0].auto_hide_delay_ms, 320);
        assert!(!s.auto_block.enabled);
        assert!(s.auto_block.apps.is_empty());

        // 重新持久化后字段已补齐，再次加载不再触发回填。
        persist(&c, &s).unwrap();
        let reloaded = load(&c, &data_dir, "1.3.0").unwrap();
        assert_eq!(reloaded.pods[0].panel_material, "acrylic");
        assert!((reloaded.pods[0].panel_opacity - 0.8).abs() < 1e-9);
    }

    #[test]
    fn panel_and_auto_block_fields_validate() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data").to_string_lossy().to_string();

        for good in ["acrylic", "plain"] {
            let mut candidate = pod(1, &tmp.path().join("stage"));
            candidate.material = good.into();
            candidate.panel_material = good.into();
            let settings = Settings {
                pods: vec![candidate],
                ..Settings::default()
            };
            assert!(validate(&settings, &data_dir).is_ok(), "材质 {good} 应有效");
        }
        for bad in ["blur", "mica", "glass", "frosted", "MICA", ""] {
            let mut candidate = pod(1, &tmp.path().join("stage"));
            candidate.material = bad.into();
            let settings = Settings {
                pods: vec![candidate],
                ..Settings::default()
            };
            assert!(validate(&settings, &data_dir).is_err(), "材质 {bad} 应无效");
        }

        let mut broken = pod(1, &tmp.path().join("stage"));
        broken.panel_opacity = 0.05;
        let settings = Settings {
            pods: vec![broken],
            ..Settings::default()
        };
        assert!(validate(&settings, &data_dir).is_err());

        let mut broken = pod(1, &tmp.path().join("stage"));
        broken.auto_hide_delay_ms = 5001;
        let settings = Settings {
            pods: vec![broken],
            ..Settings::default()
        };
        assert!(validate(&settings, &data_dir).is_err());

        let blocked = Settings {
            auto_block: AutoBlock {
                enabled: true,
                apps: (0..65).map(|i| format!("app{i}.exe")).collect(),
            },
            ..Default::default()
        };
        assert!(validate(&blocked, &data_dir).is_err());

        let blank = Settings {
            auto_block: AutoBlock {
                enabled: true,
                apps: vec!["   ".into()],
            },
            ..Default::default()
        };
        assert!(validate(&blank, &data_dir).is_err());
    }

    #[test]
    fn merge_accepts_auto_block_patch() {
        let c = conn();
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data").to_string_lossy().to_string();
        db::kv_set(&c, KEY, r#"{"theme":"system","pods":[]}"#).unwrap();
        let s = merge_persist(
            &c,
            serde_json::json!({"autoBlock":{"enabled":true,"apps":["Game.exe"]}}),
            &data_dir,
            "1.3.0",
        )
        .unwrap();
        assert!(s.auto_block.enabled);
        assert_eq!(s.auto_block.apps, vec!["Game.exe".to_string()]);
    }

    #[test]
    fn legacy_materials_migrate_to_acrylic() {
        let c = conn();
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data").to_string_lossy().to_string();
        // 两个匣必须使用不同的暂存文件夹，否则 validate 会因文件夹重复而失败。
        let stage = tmp.path().join("stage").to_string_lossy().to_string();
        let stage_two = tmp.path().join("stage-two").to_string_lossy().to_string();
        db::kv_set(
            &c,
            KEY,
            &serde_json::json!({
                "theme": "system",
                "pods": [
                    {
                        "id": 1, "name": "匣", "edge": "left", "stagingFolder": stage,
                        "material": "blur", "panelMaterial": "blur"
                    },
                    {
                        "id": 2, "name": "匣二", "edge": "right", "stagingFolder": stage_two,
                        "material": "mica", "panelMaterial": "mica"
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();
        let s = load(&c, &data_dir, "1.3.0").unwrap();
        // 边缘浮动条材质已废弃（固定普通）；浮动面板的 blur 与 mica 都迁移为亚克力。
        assert_eq!(s.pods[0].material, "plain");
        assert_eq!(s.pods[0].panel_material, "acrylic");
        assert_eq!(s.pods[1].panel_material, "acrylic");
        // 迁移结果合法，可直接通过校验。
        assert!(validate(&s, &data_dir).is_ok());
    }

    #[test]
    fn pod_upsert_delete() {
        let c = conn();
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data").to_string_lossy().to_string();
        let pod = pod(1, &tmp.path().join("stage"));
        let mut current = load(&c, &data_dir, "0.4.0").unwrap();
        upsert_pod_from(&c, &mut current, &pod, &data_dir).unwrap();
        assert_eq!(load(&c, &data_dir, "0.4.0").unwrap().pods.len(), 1);
        delete_pod(&c, 1, &data_dir, "0.4.0").unwrap();
        assert!(load(&c, &data_dir, "0.4.0").unwrap().pods.is_empty());
    }

    #[test]
    fn validate_rejects_data_dir_overlap_both_directions() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let mut settings = Settings::default();
        settings.pods.push(pod(1, &data_dir.join("stage")));
        assert!(validate(&settings, &data_dir.to_string_lossy()).is_err());

        settings.pods[0] = pod(1, tmp.path());
        assert!(validate(&settings, &data_dir.to_string_lossy()).is_err());
    }

    #[test]
    fn validate_rejects_equal_or_nested_pod_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let stage = tmp.path().join("stage");
        let settings = Settings {
            pods: vec![pod(1, &stage), pod(2, &stage.join("nested"))],
            ..Settings::default()
        };
        assert!(validate(&settings, &data_dir.to_string_lossy()).is_err());
    }

    #[test]
    fn validate_allows_legacy_disabled_pod_without_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let mut disabled = pod(1, &tmp.path().join("unused"));
        disabled.enabled = false;
        disabled.staging_folder.clear();
        let settings = Settings {
            pods: vec![disabled],
            ..Settings::default()
        };
        assert!(validate(&settings, &tmp.path().join("data").to_string_lossy()).is_ok());
    }

    #[test]
    fn pod_ids_are_monotonic_even_before_persisting_pod() {
        let c = conn();
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data").to_string_lossy().to_string();
        let current = load(&c, &data_dir, "0.4.0").unwrap();
        assert_eq!(next_pod_id_from(&c, &current).unwrap(), 1);
        assert_eq!(next_pod_id_from(&c, &current).unwrap(), 2);
    }

    #[test]
    fn pod_deserializes_without_id() {
        // 前端创建匣时不携带 id（由后端分配），缺省字段应成功反序列化
        let v = serde_json::json!({
            "name": "我的匣",
            "edge": "right",
            "stagingFolder": "D:\\暂存",
        });
        let pod: Pod = serde_json::from_value(v).unwrap();
        assert_eq!(pod.id, 0);
        assert_eq!(pod.name, "我的匣");
        assert_eq!(pod.edge, "right");
        assert_eq!(pod.staging_folder, "D:\\暂存");
        assert_eq!(pod.offset, 0.5); // 来自 Default
        assert!(pod.enabled);
    }

    #[test]
    fn bar_appearance_fields_validate() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data").to_string_lossy().to_string();

        let mut pod = pod(1, &tmp.path().join("stage"));
        pod.bar_width = 44;
        pod.corner_radius = 22;
        pod.border_color = String::new();
        pod.border_opacity = 1.0;
        let settings = Settings {
            pods: vec![pod.clone()],
            ..Settings::default()
        };
        assert!(validate(&settings, &data_dir).is_ok());

        for (field, value) in [
            ("bar_width", serde_json::json!(27u64)),
            ("bar_width", serde_json::json!(97u64)),
            ("bar_length", serde_json::json!(99u64)),
            ("bar_length", serde_json::json!(501u64)),
            ("panel_width", serde_json::json!(409u64)),
            ("panel_width", serde_json::json!(601u64)),
            ("corner_radius", serde_json::json!(65u64)),
            ("border_opacity", serde_json::json!(1.5)),
            ("border_opacity", serde_json::json!(-0.1)),
        ] {
            let mut broken = pod.clone();
            match field {
                "bar_width" => broken.bar_width = value.as_u64().unwrap() as u32,
                "bar_length" => broken.bar_length = value.as_u64().unwrap() as u32,
                "panel_width" => broken.panel_width = value.as_u64().unwrap() as u32,
                "corner_radius" => broken.corner_radius = value.as_u64().unwrap() as u32,
                "border_opacity" => broken.border_opacity = value.as_f64().unwrap(),
                _ => unreachable!(),
            }
            let settings = Settings {
                pods: vec![broken],
                ..Settings::default()
            };
            assert!(
                validate(&settings, &data_dir).is_err(),
                "{field} = {value} 应无效"
            );
        }

        for good in ["#fff", "#80FFaa", "#11223344", ""] {
            let mut candidate = pod.clone();
            candidate.border_color = good.into();
            let settings = Settings {
                pods: vec![candidate],
                ..Settings::default()
            };
            assert!(
                validate(&settings, &data_dir).is_ok(),
                "边框色 {good} 应有效"
            );
        }
        for bad in ["red", "#12", "#12345", "#1234567", "80ffaa", "#80 ffaa"] {
            let mut candidate = pod.clone();
            candidate.border_color = bad.into();
            let settings = Settings {
                pods: vec![candidate],
                ..Settings::default()
            };
            assert!(
                validate(&settings, &data_dir).is_err(),
                "边框色 {bad} 应无效"
            );
        }

        // 浮动条填充色：合法 hex 与空串（跟随主题）均可，非法值被拒绝。
        for good in ["#fff", "#80FFaa", "#11223344", ""] {
            let mut candidate = pod.clone();
            candidate.bar_color = good.into();
            let settings = Settings {
                pods: vec![candidate],
                ..Settings::default()
            };
            assert!(
                validate(&settings, &data_dir).is_ok(),
                "浮动条填充色 {good} 应有效"
            );
        }
        let mut candidate = pod.clone();
        candidate.bar_color = "red".into();
        let settings = Settings {
            pods: vec![candidate],
            ..Settings::default()
        };
        assert!(validate(&settings, &data_dir).is_err(), "非法填充色应无效");
    }
}
