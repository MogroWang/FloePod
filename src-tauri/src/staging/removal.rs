use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::db::{self, StagedItem};
use crate::events;
use crate::operations::{self, CompensationDraft, OperationDraft, OperationItemDraft};
use crate::security;
use crate::settings::{self};
use crate::state::AppState;

use super::context::*;
pub fn list_pod_items(app: &AppHandle, pod_id: u64) -> Result<Vec<StagedItem>, String> {
    security::require_unlocked(app, pod_id)?;
    let state = app.state::<AppState>();
    let connection = state.db.lock().unwrap();
    db::items_of_pod(&connection, pod_id as i64)
}

pub fn remove_items(app: AppHandle, ids: Vec<i64>, delete_files: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _permit = state.tasks.enter()?;
    let _operation = state.file_ops.lock().unwrap();
    let (current, items) = {
        let connection = state.db.lock().unwrap();
        (
            load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, &ids)?,
        )
    };
    security::require_items_unlocked(&app, &items)?;
    let validated: Vec<(&StagedItem, PathBuf)> = if delete_files {
        settings::validate(&current, &data_dir(&state))?;
        validate_item_pods(&current, &state, &items)?;
        items
            .iter()
            .map(|item| item_path(item, &current).map(|path| (item, path)))
            .collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };

    let mut removed_ids = Vec::new();
    let mut failed = Vec::new();
    let mut operation_items = Vec::new();
    if delete_files {
        for (item, path) in validated {
            let item_snapshot = operations::snapshot(item);
            match fs::symlink_metadata(&path) {
                Ok(_) => match operations::remove_to_undo_store(&state, item, &path) {
                    Ok(quarantine) => {
                        removed_ids.push(item.id);
                        operation_items.push(OperationItemDraft {
                            item_id: Some(item.id),
                            name: item.name.clone(),
                            source_path: Some(path.to_string_lossy().to_string()),
                            target_path: Some(quarantine.to_string_lossy().to_string()),
                            action: "remove".into(),
                            status: "completed".into(),
                            error: None,
                            snapshot: item_snapshot,
                            compensation: Some(CompensationDraft {
                                kind: "restore_removed_file".into(),
                                source_path: Some(quarantine.to_string_lossy().to_string()),
                                target_path: Some(path.to_string_lossy().to_string()),
                                expected_signature: operations::signature(&quarantine).ok(),
                            }),
                        });
                    }
                    Err(error) => {
                        failed.push(format!("{}: {error}", item.name));
                        operation_items.push(OperationItemDraft {
                            item_id: Some(item.id),
                            name: item.name.clone(),
                            source_path: Some(path.to_string_lossy().to_string()),
                            target_path: None,
                            action: "remove".into(),
                            status: "failed".into(),
                            error: Some(error),
                            snapshot: item_snapshot,
                            compensation: None,
                        });
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    removed_ids.push(item.id);
                    operation_items.push(OperationItemDraft {
                        item_id: Some(item.id),
                        name: item.name.clone(),
                        source_path: Some(path.to_string_lossy().to_string()),
                        target_path: None,
                        action: "remove".into(),
                        status: "stale".into(),
                        error: Some("文件已不存在，仅清理索引".into()),
                        snapshot: item_snapshot,
                        compensation: None,
                    });
                }
                Err(error) => {
                    let message = error.to_string();
                    failed.push(format!("{}: {message}", item.name));
                    operation_items.push(OperationItemDraft {
                        item_id: Some(item.id),
                        name: item.name.clone(),
                        source_path: Some(path.to_string_lossy().to_string()),
                        target_path: None,
                        action: "remove".into(),
                        status: "failed".into(),
                        error: Some(message),
                        snapshot: item_snapshot,
                        compensation: None,
                    });
                }
            }
        }
    } else {
        for item in &items {
            removed_ids.push(item.id);
            operation_items.push(OperationItemDraft {
                item_id: Some(item.id),
                name: item.name.clone(),
                source_path: Some(item.staging_path.clone()),
                target_path: None,
                action: "unlink".into(),
                status: "completed".into(),
                error: None,
                snapshot: operations::snapshot(item),
                compensation: Some(CompensationDraft {
                    kind: "restore_record".into(),
                    source_path: Some(item.staging_path.clone()),
                    target_path: None,
                    expected_signature: None,
                }),
            });
        }
    }

    let removed: HashSet<_> = removed_ids.iter().copied().collect();
    let pod_ids: HashSet<_> = items
        .iter()
        .filter(|item| removed.contains(&item.id))
        .map(|item| item.pod_id)
        .collect();
    if !removed_ids.is_empty() {
        let mut connection = state.db.lock().unwrap();
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        db::delete_items_by_ids(&transaction, &removed_ids)?;
        transaction.commit().map_err(|error| error.to_string())?;
        state.mark_staged();
    }
    for pod_id in pod_ids {
        events::emit_items_changed(&app, pod_id as u64);
    }
    if !operation_items.is_empty() {
        let has_compensation = operation_items
            .iter()
            .any(|item| item.compensation.is_some());
        let history = OperationDraft {
            kind: "remove".into(),
            pod_id: items.first().map(|item| item.pod_id),
            summary: format!(
                "从暂存中移出 {} 项{}",
                removed_ids.len(),
                if delete_files {
                    "（可撤销）"
                } else {
                    "（保留文件）"
                }
            ),
            status: if failed.is_empty() {
                "completed".into()
            } else {
                "partial".into()
            },
            undoable_until: has_compensation
                .then(|| db::now_ms().saturating_add(operations::BASIC_UNDO_MS)),
            metadata: serde_json::json!({}),
            items: operation_items,
        };
        if let Err(error) = operations::record(&state.db.lock().unwrap(), history) {
            crate::logging::write(&format!("[operations] 记录移出操作失败: {error}"));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("部分文件无法进入可撤销区：{}", failed.join("；")))
    }
}
