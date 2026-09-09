//! 持久化操作时间线、补偿动作与 24 小时基础撤销。
//!
//! 文件操作先完成自身的原子提交，再把可逆步骤写入 operations / operation_items /
//! compensations。历史写入失败不能反向破坏已经成功的文件操作，因此调用方应记录
//! 日志并把操作结果照常返回；撤销则始终保守校验文件身份，内容已变化时拒绝删除。

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use tauri::{AppHandle, Manager};

use crate::db::{self, StagedItem};
use crate::events;
use crate::file_ops;
use crate::security;
use crate::state::AppState;

use super::identity::verify_signature;
use super::model::*;
use super::store::{load_compensations, operation_deadline, operation_pod_ids};
fn unique_restore_target(target: &Path) -> Result<PathBuf, String> {
    match fs::symlink_metadata(target) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(target.to_path_buf()),
        Err(error) => return Err(format!("无法检查恢复目标: {error}")),
        Ok(_) => {}
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

pub(super) fn move_for_restore(source: &Path, target: &Path) -> Result<PathBuf, String> {
    let target = unique_restore_target(target)?;
    let parent = target
        .parent()
        .ok_or_else(|| format!("恢复路径没有父目录: {}", target.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("无法创建恢复目录 {}: {error}", parent.display()))?;
    match file_ops::rename_new(source, &target) {
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
                format!(
                    "恢复副本已保留于 {}，但无法清理原位置 {}: {error}",
                    target.display(),
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
    let _permit = state.tasks.enter()?;
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
