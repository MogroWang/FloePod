//! 持久化操作时间线、补偿动作与 24 小时基础撤销。
//!
//! 文件操作先完成自身的原子提交，再把可逆步骤写入 operations / operation_items /
//! compensations。历史写入失败不能反向破坏已经成功的文件操作，因此调用方应记录
//! 日志并把操作结果照常返回；撤销则始终保守校验文件身份，内容已变化时拒绝删除。

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::db::{self, StagedItem};
use crate::security;
use crate::state::AppState;

use super::model::*;
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

pub(super) fn operation_pod_ids(conn: &Connection, operation_id: i64) -> Result<Vec<u64>, String> {
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

pub(super) fn list_from(
    conn: &Connection,
    since: i64,
    limit: u32,
) -> Result<Vec<OperationEntry>, String> {
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

pub(super) fn load_compensations(
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

pub(super) fn operation_deadline(
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
