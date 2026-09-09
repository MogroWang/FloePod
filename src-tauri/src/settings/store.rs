use super::{
    migration,
    model::{Pod, Settings},
    validation::validate,
};
use crate::db;
use rusqlite::Connection;

pub const KEY: &str = "app";
const NEXT_POD_ID_KEY: &str = "next_pod_id";

pub fn load(conn: &Connection, data_dir: &str, version: &str) -> Result<Settings, String> {
    let mut s: Settings = match db::kv_get(conn, KEY)? {
        Some(json) => {
            let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
            migration::decode(v)?
        }
        None => Settings::default(),
    };
    crate::policy::apply_to_settings(&mut s)?;
    s.version = version.to_string();
    s.data_dir = data_dir.to_string();
    Ok(s)
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
    let mut candidate = migration::decode(raw)?;
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
    let mut candidate = current.clone();
    if let Some(existing) = candidate.pods.iter_mut().find(|p| p.id == pod.id) {
        *existing = pod.clone();
    } else {
        candidate.pods.push(pod.clone());
    }
    validate(&candidate, data_dir)?;
    persist(conn, &candidate)?;
    *current = candidate;
    Ok(())
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
