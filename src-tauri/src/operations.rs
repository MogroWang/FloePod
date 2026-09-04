//! 持久化操作时间线、补偿动作与 24 小时基础撤销。
//!
//! 文件操作先完成自身的原子提交，再把可逆步骤写入 operations / operation_items /
//! compensations。历史写入失败不能反向破坏已经成功的文件操作，因此调用方应记录
//! 日志并把操作结果照常返回；撤销则始终保守校验文件身份，内容已变化时拒绝删除。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::db::{self, StagedItem};
use crate::events;
use crate::file_ops;
use crate::security;
use crate::staging;
use crate::state::AppState;

pub const BASIC_UNDO_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone)]
pub struct CompensationDraft {
    pub kind: String,
    pub source_path: Option<String>,
    pub target_path: Option<String>,
    pub expected_signature: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OperationItemDraft {
    pub item_id: Option<i64>,
    pub name: String,
    pub source_path: Option<String>,
    pub target_path: Option<String>,
    pub action: String,
    pub status: String,
    pub error: Option<String>,
    pub snapshot: Option<String>,
    pub compensation: Option<CompensationDraft>,
}

#[derive(Debug, Clone)]
pub struct OperationDraft {
    pub kind: String,
    pub pod_id: Option<i64>,
    pub summary: String,
    pub status: String,
    pub undoable_until: Option<i64>,
    pub metadata: Value,
    pub items: Vec<OperationItemDraft>,
}

impl OperationDraft {
    pub fn completed(
        kind: impl Into<String>,
        pod_id: Option<i64>,
        summary: impl Into<String>,
        metadata: Value,
        items: Vec<OperationItemDraft>,
    ) -> Self {
        Self {
            kind: kind.into(),
            pod_id,
            summary: summary.into(),
            status: "completed".into(),
            undoable_until: Some(db::now_ms().saturating_add(BASIC_UNDO_MS)),
            metadata,
            items,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationItemEntry {
    pub id: i64,
    pub item_id: Option<i64>,
    pub name: String,
    pub source_path: Option<String>,
    pub target_path: Option<String>,
    pub action: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationEntry {
    pub id: i64,
    pub kind: String,
    pub pod_id: Option<i64>,
    pub summary: String,
    pub status: String,
    pub created_at: i64,
    pub undoable_until: Option<i64>,
    pub undone_at: Option<i64>,
    pub undoable: bool,
    pub retryable: bool,
    pub items: Vec<OperationItemEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResult {
    pub operation_id: i64,
    pub restored: usize,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryResult {
    pub operation_id: i64,
    pub kind: String,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationPreview {
    pub title: String,
    pub details: Vec<String>,
    pub warnings: Vec<String>,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone)]
struct StoredCompensation {
    id: i64,
    item_id: Option<i64>,
    pod_id: Option<i64>,
    name: String,
    kind: String,
    source_path: Option<String>,
    target_path: Option<String>,
    expected_signature: Option<String>,
    snapshot: Option<String>,
}

pub fn signature(path: &Path) -> Result<String, String> {
    let mut hash = 0xcbf29ce484222325u64;
    signature_path(path, path, &mut hash)?;
    Ok(format!("{hash:016x}"))
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= *byte as u64;
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn modified_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn signature_path(root: &Path, path: &Path, hash: &mut u64) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("无法读取 {}: {error}", path.display()))?;
    if file_ops::is_reparse_or_symlink(&metadata) {
        return Err(format!(
            "不对符号链接或目录重解析点执行撤销: {}",
            path.display()
        ));
    }
    let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    hash_bytes(hash, relative.as_bytes());
    hash_bytes(hash, &metadata.len().to_le_bytes());
    hash_bytes(hash, &modified_ms(&metadata).to_le_bytes());
    hash_bytes(hash, &[metadata.is_dir() as u8]);
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)
            .map_err(|error| format!("无法读取目录 {}: {error}", path.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("无法枚举目录 {}: {error}", path.display()))?;
        children.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
        for child in children {
            signature_path(root, &child.path(), hash)?;
        }
    }
    Ok(())
}

pub fn snapshot(item: &StagedItem) -> Option<String> {
    serde_json::to_string(item).ok()
}

pub fn record(conn: &Connection, draft: OperationDraft) -> Result<i64, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let created_at = db::now_ms();
    tx.execute(
        "INSERT INTO operations
         (kind, pod_id, summary, status, created_at, undoable_until, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            draft.kind,
            draft.pod_id,
            draft.summary,
            draft.status,
            created_at,
            draft.undoable_until,
            draft.metadata.to_string(),
        ],
    )
    .map_err(|error| error.to_string())?;
    let operation_id = tx.last_insert_rowid();
    for item in draft.items {
        tx.execute(
            "INSERT INTO operation_items
             (operation_id, item_id, name, source_path, target_path, action, status, error, snapshot)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                operation_id,
                item.item_id,
                item.name,
                item.source_path,
                item.target_path,
                item.action,
                item.status,
                item.error,
                item.snapshot,
            ],
        )
        .map_err(|error| error.to_string())?;
        let operation_item_id = tx.last_insert_rowid();
        if let Some(compensation) = item.compensation {
            tx.execute(
                "INSERT INTO compensations
                 (operation_item_id, kind, source_path, target_path, expected_signature)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    operation_item_id,
                    compensation.kind,
                    compensation.source_path,
                    compensation.target_path,
                    compensation.expected_signature,
                ],
            )
            .map_err(|error| error.to_string())?;
        }
    }
    tx.commit().map_err(|error| error.to_string())?;
    Ok(operation_id)
}

pub fn list(app: &AppHandle, hours: u32, limit: u32) -> Result<Vec<OperationEntry>, String> {
    let state = app.state::<AppState>();
    let hours = hours.clamp(1, 24 * 365 * 10) as i64;
    let since = db::now_ms().saturating_sub(hours.saturating_mul(60 * 60 * 1_000));
    let (mut entries, operation_pods) = {
        let conn = state.db.lock().unwrap();
        let entries = list_from(&conn, since, limit.clamp(1, 500))?;
        let operation_pods = entries
            .iter()
            .map(|entry| Ok((entry.id, operation_pod_ids(&conn, entry.id)?)))
            .collect::<Result<HashMap<_, _>, String>>()?;
        (entries, operation_pods)
    };
    let mut locked = HashMap::new();
    for pod_id in operation_pods.values().flatten().copied() {
        locked
            .entry(pod_id)
            .or_insert_with(|| security::is_locked(app, pod_id));
    }
    for entry in &mut entries {
        let contains_locked = operation_pods
            .get(&entry.id)
            .is_some_and(|ids| ids.iter().any(|id| locked.get(id) == Some(&true)));
        if contains_locked {
            entry.summary = "敏感匣操作（已锁定）".into();
            entry.undoable = false;
            entry.retryable = false;
            for item in &mut entry.items {
                item.name = "已锁定项目".into();
                item.source_path = None;
                item.target_path = None;
                item.error = None;
            }
        }
    }
    Ok(entries)
}

fn operation_pod_ids(conn: &Connection, operation_id: i64) -> Result<Vec<u64>, String> {
    let mut statement = conn
        .prepare(
            "SELECT o.pod_id, oi.snapshot
             FROM operations o
             LEFT JOIN operation_items oi ON oi.operation_id = o.id
             WHERE o.id = ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![operation_id], |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut ids = Vec::new();
    for row in rows {
        let (fallback, snapshot) = row.map_err(|error| error.to_string())?;
        let from_snapshot = snapshot
            .as_deref()
            .and_then(|value| serde_json::from_str::<StagedItem>(value).ok())
            .map(|item| item.pod_id);
        if let Some(id) = from_snapshot
            .or(fallback)
            .and_then(|id| u64::try_from(id).ok())
        {
            ids.push(id);
        }
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

fn list_from(conn: &Connection, since: i64, limit: u32) -> Result<Vec<OperationEntry>, String> {
    let now = db::now_ms();
    let mut statement = conn
        .prepare(
            "SELECT id, kind, pod_id, summary, status, created_at, undoable_until, undone_at,
                    metadata
             FROM operations WHERE created_at >= ?1 ORDER BY created_at DESC LIMIT ?2",
        )
        .map_err(|error| error.to_string())?;
    let operations = statement
        .query_map(params![since, limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut result = Vec::with_capacity(operations.len());
    for (id, kind, pod_id, summary, status, created_at, undoable_until, undone_at, metadata) in
        operations
    {
        let mut item_statement = conn
            .prepare(
                "SELECT id, item_id, name, source_path, target_path, action, status, error
                 FROM operation_items WHERE operation_id = ?1 ORDER BY id",
            )
            .map_err(|error| error.to_string())?;
        let items = item_statement
            .query_map(params![id], |row| {
                Ok(OperationItemEntry {
                    id: row.get(0)?,
                    item_id: row.get(1)?,
                    name: row.get(2)?,
                    source_path: row.get(3)?,
                    target_path: row.get(4)?,
                    action: row.get(5)?,
                    status: row.get(6)?,
                    error: row.get(7)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let retryable = serde_json::from_str::<Value>(&metadata)
            .ok()
            .and_then(|value| value.get("retry").cloned())
            .is_some();
        let pending_compensations: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM compensations c
                 JOIN operation_items oi ON oi.id = c.operation_item_id
                 WHERE oi.operation_id = ?1 AND c.status = 'pending'",
                params![id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let undoable = undone_at.is_none()
            && status != "undone"
            && undoable_until.is_some_and(|deadline| deadline >= now)
            && pending_compensations > 0;
        result.push(OperationEntry {
            id,
            kind,
            pod_id,
            summary,
            status,
            created_at,
            undoable_until,
            undone_at,
            undoable,
            retryable,
            items,
        });
    }
    Ok(result)
}

fn load_compensations(
    conn: &Connection,
    operation_id: i64,
) -> Result<Vec<StoredCompensation>, String> {
    let mut statement = conn
        .prepare(
            "SELECT c.id, oi.item_id, o.pod_id, oi.name, c.kind, c.source_path,
                    c.target_path, c.expected_signature, oi.snapshot
             FROM compensations c
             JOIN operation_items oi ON oi.id = c.operation_item_id
             JOIN operations o ON o.id = oi.operation_id
             WHERE o.id = ?1 AND c.status = 'pending' ORDER BY c.id DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![operation_id], |row| {
            Ok(StoredCompensation {
                id: row.get(0)?,
                item_id: row.get(1)?,
                pod_id: row.get(2)?,
                name: row.get(3)?,
                kind: row.get(4)?,
                source_path: row.get(5)?,
                target_path: row.get(6)?,
                expected_signature: row.get(7)?,
                snapshot: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let result = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(result)
}

fn operation_deadline(
    conn: &Connection,
    operation_id: i64,
) -> Result<(String, Option<i64>), String> {
    conn.query_row(
        "SELECT status, undoable_until FROM operations WHERE id = ?1",
        params![operation_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "操作记录不存在".to_string())
}

fn verify_signature(path: &Path, expected: Option<&str>) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = signature(path)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "文件在操作后已经变化，为避免误删已拒绝撤销: {}",
            path.display()
        ))
    }
}

fn unique_restore_target(target: &Path) -> Result<PathBuf, String> {
    if fs::symlink_metadata(target).is_err() {
        return Ok(target.to_path_buf());
    }
    let parent = target
        .parent()
        .ok_or_else(|| format!("恢复路径没有父目录: {}", target.display()))?;
    let name = target
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .ok_or_else(|| format!("恢复路径无效: {}", target.display()))?;
    file_ops::unique_target(parent, &name, &mut HashSet::new())
}

fn move_for_restore(source: &Path, target: &Path) -> Result<PathBuf, String> {
    let target = unique_restore_target(target)?;
    let parent = target
        .parent()
        .ok_or_else(|| format!("恢复路径没有父目录: {}", target.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("无法创建恢复目录 {}: {error}", parent.display()))?;
    match fs::rename(source, &target) {
        Ok(()) => Ok(target),
        Err(rename_error) => {
            file_ops::copy_path(source, &target).map_err(|copy_error| {
                format!(
                    "无法把 {} 恢复到 {}（重命名: {rename_error}；复制: {copy_error}）",
                    source.display(),
                    target.display()
                )
            })?;
            file_ops::remove_path(source).map_err(|error| {
                let _ = file_ops::remove_path(&target);
                format!(
                    "恢复副本已生成，但无法清理原位置 {}: {error}",
                    source.display()
                )
            })?;
            Ok(target)
        }
    }
}

fn restore_snapshot(
    conn: &Connection,
    snapshot: Option<&str>,
    actual: Option<&Path>,
) -> Result<(), String> {
    let snapshot = snapshot.ok_or_else(|| "操作记录缺少条目快照".to_string())?;
    let mut item: StagedItem = serde_json::from_str(snapshot).map_err(|error| error.to_string())?;
    if let Some(actual) = actual {
        item.staging_path = actual.to_string_lossy().to_string();
        item.name = actual
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or(item.name);
        item.size = fs::metadata(actual)
            .map(|metadata| {
                if metadata.is_dir() {
                    0
                } else {
                    metadata.len() as i64
                }
            })
            .unwrap_or(item.size);
    }
    match db::insert_item(conn, &item) {
        Ok(_) => Ok(()),
        Err(error) => match db::find_by_path(conn, &item.staging_path)? {
            Some(existing) if existing.pod_id == item.pod_id => Ok(()),
            _ => Err(error),
        },
    }
}

fn apply_compensation(state: &AppState, compensation: &StoredCompensation) -> Result<(), String> {
    match compensation.kind.as_str() {
        "delete_staged_copy" => {
            let target = PathBuf::from(
                compensation
                    .target_path
                    .as_deref()
                    .ok_or_else(|| "撤销记录缺少暂存目标".to_string())?,
            );
            match fs::symlink_metadata(&target) {
                Ok(_) => {
                    verify_signature(&target, compensation.expected_signature.as_deref())?;
                    trash::delete(&target)
                        .map_err(|error| format!("无法把暂存副本移入回收站: {error}"))?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
            if let Some(item_id) = compensation.item_id {
                db::delete_items_by_ids(&state.db.lock().unwrap(), &[item_id])?;
            }
            Ok(())
        }
        "restore_stage_move" => {
            let current = PathBuf::from(
                compensation
                    .target_path
                    .as_deref()
                    .ok_or_else(|| "撤销记录缺少当前路径".to_string())?,
            );
            let original = PathBuf::from(
                compensation
                    .source_path
                    .as_deref()
                    .ok_or_else(|| "撤销记录缺少原路径".to_string())?,
            );
            verify_signature(&current, compensation.expected_signature.as_deref())?;
            move_for_restore(&current, &original)?;
            if let Some(item_id) = compensation.item_id {
                db::delete_items_by_ids(&state.db.lock().unwrap(), &[item_id])?;
            }
            Ok(())
        }
        "delete_export_copy" => {
            let target = PathBuf::from(
                compensation
                    .target_path
                    .as_deref()
                    .ok_or_else(|| "撤销记录缺少导出目标".to_string())?,
            );
            match fs::symlink_metadata(&target) {
                Ok(_) => {
                    verify_signature(&target, compensation.expected_signature.as_deref())?;
                    trash::delete(&target)
                        .map_err(|error| format!("无法把导出副本移入回收站: {error}"))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            }
        }
        "restore_export_move" | "restore_removed_file" => {
            let current = PathBuf::from(
                compensation
                    .source_path
                    .as_deref()
                    .ok_or_else(|| "撤销记录缺少恢复来源".to_string())?,
            );
            let target = PathBuf::from(
                compensation
                    .target_path
                    .as_deref()
                    .ok_or_else(|| "撤销记录缺少恢复目标".to_string())?,
            );
            verify_signature(&current, compensation.expected_signature.as_deref())?;
            let actual = move_for_restore(&current, &target)?;
            restore_snapshot(
                &state.db.lock().unwrap(),
                compensation.snapshot.as_deref(),
                Some(&actual),
            )
        }
        "restore_record" => restore_snapshot(
            &state.db.lock().unwrap(),
            compensation.snapshot.as_deref(),
            None,
        ),
        other => Err(format!("未知撤销动作: {other}")),
    }
}

pub fn undo(app: AppHandle, operation_id: i64) -> Result<UndoResult, String> {
    let state = app.state::<AppState>();
    let pod_ids = operation_pod_ids(&state.db.lock().unwrap(), operation_id)?;
    for pod_id in pod_ids {
        security::require_unlocked(&app, pod_id)?;
    }
    let _file_operation = state.file_ops.lock().unwrap();
    let (status, deadline) = {
        let conn = state.db.lock().unwrap();
        operation_deadline(&conn, operation_id)?
    };
    if status == "undone" {
        return Err("该操作已经撤销".into());
    }
    if deadline.is_none_or(|value| value < db::now_ms()) {
        return Err("该操作已超过基础撤销期限".into());
    }
    let compensations = {
        let conn = state.db.lock().unwrap();
        load_compensations(&conn, operation_id)?
    };
    if compensations.is_empty() {
        return Err("该操作没有可执行的撤销步骤".into());
    }

    let mut restored = 0usize;
    let mut failed = Vec::new();
    let mut changed_pods = HashSet::new();
    for compensation in compensations {
        if let Some(pod_id) = compensation.pod_id {
            changed_pods.insert(pod_id);
        }
        let outcome = apply_compensation(&state, &compensation);
        let (next_status, error) = match outcome {
            Ok(()) => {
                restored += 1;
                ("completed", None)
            }
            Err(error) => {
                failed.push(format!("{}：{error}", compensation.name));
                // 保持 pending，用户修复占用/冲突后可以再次点击撤销；error 仅作提示。
                ("pending", Some(error))
            }
        };
        state
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE compensations SET status = ?1, error = ?2 WHERE id = ?3",
                params![next_status, error, compensation.id],
            )
            .map_err(|error| error.to_string())?;
    }
    let final_status = if failed.is_empty() {
        "undone"
    } else {
        "undo_failed"
    };
    let undone_at = failed.is_empty().then(db::now_ms);
    state
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE operations SET status = ?1, undone_at = ?2 WHERE id = ?3",
            params![final_status, undone_at, operation_id],
        )
        .map_err(|error| error.to_string())?;
    state.mark_staged();
    for pod_id in changed_pods {
        events::emit_items_changed(&app, pod_id as u64);
    }
    Ok(UndoResult {
        operation_id,
        restored,
        failed,
    })
}

pub fn retry(app: AppHandle, operation_id: i64) -> Result<RetryResult, String> {
    let state = app.state::<AppState>();
    let (kind, metadata): (String, String) = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT kind, metadata FROM operations WHERE id = ?1",
            params![operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "操作记录不存在".to_string())?;
    let metadata: Value = serde_json::from_str(&metadata).map_err(|error| error.to_string())?;
    let retry = metadata
        .get("retry")
        .ok_or_else(|| "该操作没有可重试的失败项".to_string())?;
    let result = match kind.as_str() {
        "stage" => {
            let pod_id = retry
                .get("podId")
                .and_then(Value::as_u64)
                .ok_or_else(|| "重试记录缺少匣 ID".to_string())?;
            let paths = retry
                .get("paths")
                .and_then(Value::as_array)
                .ok_or_else(|| "重试记录缺少路径".to_string())?
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect();
            let action = retry
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("copy")
                .to_string();
            serde_json::to_value(crate::staging::stage_paths(
                app.clone(),
                pod_id,
                paths,
                action,
            )?)
            .map_err(|error| error.to_string())?
        }
        "export" => {
            let ids = retry
                .get("ids")
                .and_then(Value::as_array)
                .ok_or_else(|| "重试记录缺少条目".to_string())?
                .iter()
                .filter_map(Value::as_i64)
                .collect();
            let destination = retry
                .get("destination")
                .and_then(Value::as_str)
                .ok_or_else(|| "重试记录缺少目标文件夹".to_string())?
                .to_string();
            let mode = retry
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("copy")
                .to_string();
            serde_json::to_value(crate::export::export_items(
                app.clone(),
                ids,
                destination,
                mode,
                "rename".into(),
            )?)
            .map_err(|error| error.to_string())?
        }
        _ => return Err("该操作类型不支持自动重试".into()),
    };
    Ok(RetryResult {
        operation_id,
        kind,
        result,
    })
}

pub fn preview_remove(
    app: &AppHandle,
    ids: &[i64],
    delete_files: bool,
) -> Result<OperationPreview, String> {
    let state = app.state::<AppState>();
    let (settings, items) = {
        let connection = state.db.lock().unwrap();
        (
            staging::load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, ids)?,
        )
    };
    staging::validate_item_pods(&settings, &state, &items)?;
    security::require_items_unlocked(app, &items)?;
    let details = items
        .iter()
        .map(|item| format!("{} — {}", item.name, item.staging_path))
        .collect::<Vec<_>>();
    let warnings = if delete_files {
        vec!["文件会先进入 FloePod 的 24 小时可撤销区；到期清理时再移入系统回收站。".into()]
    } else {
        vec!["只移除索引，原文件仍留在暂存文件夹中。".into()]
    };
    Ok(OperationPreview {
        title: format!("将从暂存中移出 {} 项", items.len()),
        details,
        warnings,
        requires_confirmation: delete_files || items.len() > 1,
    })
}

pub fn preview_export(
    app: &AppHandle,
    ids: &[i64],
    destination: &str,
    mode: &str,
) -> Result<OperationPreview, String> {
    let state = app.state::<AppState>();
    let (settings, items) = {
        let connection = state.db.lock().unwrap();
        (
            staging::load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, ids)?,
        )
    };
    staging::validate_item_pods(&settings, &state, &items)?;
    security::require_items_unlocked(app, &items)?;
    let destination = PathBuf::from(destination);
    if !destination.is_absolute() {
        return Err("目标文件夹必须是绝对路径".into());
    }
    let mut warnings = Vec::new();
    let details = items
        .iter()
        .map(|item| {
            let target = destination.join(&item.name);
            if fs::symlink_metadata(&target).is_ok() {
                warnings.push(format!("目标已存在同名项：{}", item.name));
            }
            format!("{} → {}", item.staging_path, target.display())
        })
        .collect();
    if mode == "move" {
        warnings.push("移动完成后会从当前匣移除；24 小时内可以从操作中心恢复。".into());
    }
    Ok(OperationPreview {
        title: format!(
            "将{} {} 项",
            if mode == "move" { "移动" } else { "复制" },
            items.len()
        ),
        details,
        warnings,
        requires_confirmation: mode == "move" || items.len() > 1,
    })
}

pub fn undo_root(state: &AppState) -> PathBuf {
    state.data_dir.join("undo")
}

pub fn purge_expired(app: &AppHandle) {
    let state = app.state::<AppState>();
    let expired = {
        let conn = state.db.lock().unwrap();
        let mut statement = match conn.prepare(
            "SELECT c.id, c.source_path
             FROM compensations c
             JOIN operation_items oi ON oi.id = c.operation_item_id
             JOIN operations o ON o.id = oi.operation_id
             WHERE c.kind = 'restore_removed_file' AND c.status = 'pending'
               AND o.undoable_until IS NOT NULL AND o.undoable_until < ?1",
        ) {
            Ok(statement) => statement,
            Err(error) => {
                crate::logging::write(&format!("[operations] 读取过期撤销项失败: {error}"));
                return;
            }
        };
        let rows = match statement.query_map(params![db::now_ms()], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        }) {
            Ok(rows) => rows,
            Err(error) => {
                crate::logging::write(&format!("[operations] 枚举过期撤销项失败: {error}"));
                return;
            }
        };
        let collected = rows.filter_map(Result::ok).collect::<Vec<_>>();
        drop(statement);
        drop(conn);
        collected
    };
    for (id, path) in expired {
        let outcome = path
            .map(PathBuf::from)
            .filter(|path| path.exists())
            .map(|path| trash::delete(&path).map_err(|error| error.to_string()))
            .unwrap_or(Ok(()));
        match outcome {
            Ok(()) => {
                let _ = state.db.lock().unwrap().execute(
                    "UPDATE compensations SET status = 'expired' WHERE id = ?1",
                    params![id],
                );
            }
            Err(error) => crate::logging::write(&format!(
                "[operations] 清理过期撤销文件失败（下次启动重试）: {error}"
            )),
        }
    }
}

pub fn remove_to_undo_store(
    state: &AppState,
    item: &StagedItem,
    path: &Path,
) -> Result<PathBuf, String> {
    let batch = format!("{}-{}", db::now_ms(), std::process::id());
    let root = undo_root(state).join(batch);
    fs::create_dir_all(&root)
        .map_err(|error| format!("无法创建可撤销区 {}: {error}", root.display()))?;
    let target = file_ops::unique_target(&root, &item.name, &mut HashSet::new())?;
    move_for_restore(path, &target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        db::migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn operation_round_trip_exposes_undo_and_retry_flags() {
        let conn = connection();
        let id = record(
            &conn,
            OperationDraft::completed(
                "stage",
                Some(1),
                "复制 1 项到测试匣",
                serde_json::json!({"retry":{"podId":1,"paths":["C:\\a.txt"],"action":"copy"}}),
                vec![OperationItemDraft {
                    item_id: Some(7),
                    name: "a.txt".into(),
                    source_path: Some("C:\\a.txt".into()),
                    target_path: Some("D:\\pod\\a.txt".into()),
                    action: "copy".into(),
                    status: "completed".into(),
                    error: None,
                    snapshot: None,
                    compensation: Some(CompensationDraft {
                        kind: "delete_staged_copy".into(),
                        source_path: None,
                        target_path: Some("D:\\pod\\a.txt".into()),
                        expected_signature: Some("abc".into()),
                    }),
                }],
            ),
        )
        .unwrap();
        assert!(id > 0);
        let rows = list_from(&conn, 0, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].undoable);
        assert!(rows[0].retryable);
        assert_eq!(rows[0].items[0].name, "a.txt");
    }

    #[test]
    fn signature_changes_when_file_content_changes() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("a.txt");
        fs::write(&path, b"one").unwrap();
        let before = signature(&path).unwrap();
        fs::write(&path, b"a different value").unwrap();
        let after = signature(&path).unwrap();
        assert_ne!(before, after);
    }

    #[test]
    fn restore_uses_a_non_conflicting_name() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("quarantine.txt");
        let target = temporary.path().join("restored.txt");
        fs::write(&source, b"restored").unwrap();
        fs::write(&target, b"existing").unwrap();
        let actual = move_for_restore(&source, &target).unwrap();
        assert_ne!(actual, target);
        assert_eq!(fs::read(actual).unwrap(), b"restored");
        assert_eq!(fs::read(target).unwrap(), b"existing");
    }

    #[test]
    fn expired_operations_are_not_undoable() {
        let conn = connection();
        conn.execute(
            "INSERT INTO operations
             (kind, summary, status, created_at, undoable_until, metadata)
             VALUES ('remove', '旧操作', 'completed', 1, 2, '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO operation_items
             (operation_id, name, action, status) VALUES (1, 'a', 'remove', 'completed')",
            [],
        )
        .unwrap();
        let rows = list_from(&conn, 0, 10).unwrap();
        assert!(!rows[0].undoable);
    }
}
