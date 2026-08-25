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

impl Hotkeys {
    pub fn with_defaults() -> Self {
        Self {
            toggle_bar: d_toggle_bar(),
            collect_clipboard: d_collect_clipboard(),
            open_panel: d_open_panel(),
        }
    }
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
    pub material: String,
    pub panel_width: u32,
    pub hover_delay_ms: u64,
    pub drop_action: String,
    pub enabled: bool,
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
            material: "acrylic".into(),
            panel_width: 380,
            hover_delay_ms: 120,
            drop_action: "ask".into(),
            enabled: true,
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
            s
        }
        None => Settings::default(),
    };
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

/// Compare two persisted path spellings without requiring their drive/share to be online.
/// Runtime file operations must still call [`resolve_path`] before touching the filesystem.
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
        if !matches!(pod.material.as_str(), "acrylic" | "plain") {
            return Err(format!("匣「{}」的材质无效", pod.name));
        }
        if !(300..=520).contains(&pod.panel_width) {
            return Err(format!("匣「{}」的面板宽度无效", pod.name));
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
    s.pods.push(Pod {
        id: 1,
        name: "我的匣".into(),
        edge: edge.into(),
        monitor: String::new(),
        offset: 0.5,
        staging_folder: folder,
        opacity: v.get("opacity").and_then(|x| x.as_f64()).unwrap_or(1.0),
        material: v
            .get("material")
            .and_then(|x| x.as_str())
            .unwrap_or("acrylic")
            .into(),
        panel_width: v.get("panelWidth").and_then(|x| x.as_u64()).unwrap_or(380) as u32,
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
    });
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
            "theme" | "firstRunDone" | "autostart" | "hotkeys" => {
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
    candidate.version = version.to_string();
    candidate.data_dir = data_dir.to_string();
    validate(&candidate, data_dir)?;
    persist(conn, &candidate)?;
    Ok(candidate)
}

/* ---------- pod 增删改（读写设置并持久化） ---------- */

pub fn next_pod_id(conn: &Connection, data_dir: &str, version: &str) -> Result<u64, String> {
    let s = load(conn, data_dir, version)?;
    let floor = s
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

pub fn upsert_pod(
    conn: &Connection,
    pod: &Pod,
    data_dir: &str,
    version: &str,
) -> Result<Settings, String> {
    let mut s = load(conn, data_dir, version)?;
    if let Some(existing) = s.pods.iter_mut().find(|p| p.id == pod.id) {
        *existing = pod.clone();
    } else {
        s.pods.push(pod.clone());
    }
    validate(&s, data_dir)?;
    persist(conn, &s)?;
    Ok(s)
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
            panel_width: 380,
            hover_delay_ms: 120,
            drop_action: "ask".into(),
            enabled: true,
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
            version: "0.6.0".into(),
            data_dir: r"C:\Users\tester\AppData\Roaming\FloePod".into(),
            ..Settings::default()
        };

        let wire = serde_json::to_value(&s).unwrap();
        assert_eq!(wire["version"], "0.6.0");
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
                    "panelWidth": 512,
                    "hoverDelayMs": 600,
                    "dropAction": "move",
                    "enabled": true
                },
                {
                    "id": 19,
                    "name": "离线便携匣",
                    "edge": "bottom",
                    "monitor": "",
                    "offset": 1.0,
                    "stagingFolder": stage_two,
                    "opacity": 1.0,
                    "material": "acrylic",
                    "panelWidth": 300,
                    "hoverDelayMs": 0,
                    "dropAction": "shortcut",
                    "enabled": false
                }
            ]
        });
        db::kv_set(&c, KEY, &fixture.to_string()).unwrap();

        let loaded = load(&c, &data_dir, "0.6.0").unwrap();
        assert_eq!(loaded.pods.len(), 2);
        assert_eq!(loaded.pods[0].id, 7);
        assert_eq!(loaded.pods[0].monitor, "DISPLAY-A");
        assert_eq!(loaded.pods[0].panel_width, 512);
        assert_eq!(loaded.pods[1].drop_action, "shortcut");
        assert!(!loaded.pods[1].enabled);

        persist(&c, &loaded).unwrap();
        let stored: serde_json::Value =
            serde_json::from_str(&db::kv_get(&c, KEY).unwrap().unwrap()).unwrap();
        assert_eq!(stored, fixture);
    }

    #[test]
    fn pod_upsert_delete() {
        let c = conn();
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data").to_string_lossy().to_string();
        let pod = pod(1, &tmp.path().join("stage"));
        upsert_pod(&c, &pod, &data_dir, "0.4.0").unwrap();
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
        assert_eq!(next_pod_id(&c, &data_dir, "0.4.0").unwrap(), 1);
        assert_eq!(next_pod_id(&c, &data_dir, "0.4.0").unwrap(), 2);
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
}
