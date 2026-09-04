use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db;
use crate::events;
use crate::file_ops;
use crate::operations::{self, CompensationDraft, OperationDraft, OperationItemDraft};
use crate::policy;
use crate::security;
use crate::settings;
use crate::staging;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportIssue {
    id: i64,
    name: String,
    error: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    conflicts: Vec<String>,
    completed_ids: Vec<i64>,
    skipped_ids: Vec<i64>,
    stale_ids: Vec<i64>,
    failed: Vec<ExportIssue>,
    warnings: Vec<ExportIssue>,
}

pub fn export_items(
    app: AppHandle,
    ids: Vec<i64>,
    destination: String,
    mode: String,
    conflict_strategy: String,
) -> Result<ExportResult, String> {
    if !matches!(mode.as_str(), "copy" | "move") {
        return Err(format!("未知导出模式: {mode}"));
    }
    policy::enforce_export(&mode, false)?;
    if !matches!(
        conflict_strategy.as_str(),
        "ask" | "overwrite" | "skip" | "rename"
    ) {
        return Err(format!("未知冲突策略: {conflict_strategy}"));
    }
    let state = app.state::<AppState>();
    let _operation = state.file_ops.lock().unwrap();
    let (current, items) = {
        let connection = state.db.lock().unwrap();
        (
            staging::load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, &ids)?,
        )
    };
    if items.is_empty() {
        return Ok(ExportResult::default());
    }
    security::require_items_unlocked(&app, &items)?;
    settings::validate(&current, &staging::data_dir(&state))?;
    staging::validate_item_pods(&current, &state, &items)?;
    let sources: Vec<_> = items
        .iter()
        .map(|item| staging::item_path(item, &current).map(|path| (item, path)))
        .collect::<Result<_, _>>()?;

    let destination = PathBuf::from(destination);
    if !destination.is_absolute() {
        return Err("目标文件夹必须是绝对路径".into());
    }
    fs::create_dir_all(&destination).map_err(|error| format!("目标文件夹不可用: {error}"))?;
    let destination = settings::resolve_path(&destination)?;

    let mut conflict_keys = HashSet::new();
    let mut conflicts = Vec::new();
    for (item, _) in &sources {
        let candidate = destination.join(&item.name);
        let key = settings::path_key(&settings::resolve_path(&candidate)?);
        if fs::symlink_metadata(&candidate).is_ok() || !conflict_keys.insert(key) {
            conflicts.push(item.name.clone());
        }
    }
    if conflict_strategy == "ask" && !conflicts.is_empty() {
        return Ok(ExportResult {
            conflicts,
            ..ExportResult::default()
        });
    }

    let mut reserved = HashSet::new();
    let mut temporary_reserved = HashSet::new();
    let mut moved_ids = Vec::new();
    let mut changed_pods = HashSet::new();
    let mut result = ExportResult::default();
    let mut batch_targets = HashSet::new();
    let mut successful_targets = Vec::new();
    let auto_remove_pods = current
        .pods
        .iter()
        .filter(|pod| {
            pod.rules.enabled && pod.rules.remove_after_export
                || pod.security.enabled && pod.security.cleanup_after_export
        })
        .map(|pod| pod.id as i64)
        .collect::<HashSet<_>>();
    for (item, source) in sources {
        let issue = |error: String| ExportIssue {
            id: item.id,
            name: item.name.clone(),
            error,
        };
        match fs::symlink_metadata(&source) {
            Ok(metadata) if file_ops::is_reparse_or_symlink(&metadata) => {
                result
                    .failed
                    .push(issue("不支持符号链接或目录重解析点".into()));
                continue;
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if mode == "move" {
                    moved_ids.push(item.id);
                    changed_pods.insert(item.pod_id);
                    // 源文件不存在时只清理旧记录，不生成目标文件。
                    result.stale_ids.push(item.id);
                } else {
                    result.failed.push(issue("源文件已不存在".into()));
                }
                continue;
            }
            Err(error) => {
                result.failed.push(issue(error.to_string()));
                continue;
            }
        }

        let target = match conflict_strategy.as_str() {
            "overwrite" => destination.join(&item.name),
            "skip" => {
                if fs::symlink_metadata(destination.join(&item.name)).is_ok() {
                    result.skipped_ids.push(item.id);
                    continue;
                }
                destination.join(&item.name)
            }
            "rename" => match file_ops::unique_target(&destination, &item.name, &mut reserved) {
                Ok(target) => target,
                Err(error) => {
                    result.failed.push(issue(error));
                    continue;
                }
            },
            "ask" => destination.join(&item.name),
            _ => unreachable!(),
        };
        if let Ok(metadata) = fs::symlink_metadata(&target) {
            if file_ops::is_reparse_or_symlink(&metadata) {
                result
                    .failed
                    .push(issue("目标名称指向符号链接或目录重解析点".into()));
                continue;
            }
        }
        let resolved_target = match settings::resolve_path(&target) {
            Ok(target) => target,
            Err(error) => {
                result.failed.push(issue(error));
                continue;
            }
        };
        if !settings::path_is_within(&resolved_target, &destination)
            || settings::paths_equal(&resolved_target, &destination)
        {
            result.failed.push(issue("目标路径越出所选目录".into()));
            continue;
        }
        if !batch_targets.insert(settings::path_key(&resolved_target)) {
            result.failed.push(issue("批次内存在重复目标名称".into()));
            continue;
        }
        if let Err(error) = file_ops::ensure_distinct_target(&source, &target) {
            result.failed.push(issue(error));
            continue;
        }
        let copy = match file_ops::copy_for_export(
            &source,
            &target,
            &destination,
            conflict_strategy == "overwrite",
            &mut temporary_reserved,
        ) {
            Ok(copy) => copy,
            Err(error) => {
                result.failed.push(issue(format!("导出失败: {error}")));
                continue;
            }
        };
        result.completed_ids.push(item.id);
        if let Some(warning) = copy.warning {
            result.warnings.push(issue(warning));
        }
        let remove_source = mode == "move" || auto_remove_pods.contains(&item.pod_id);
        let mut source_removed = false;
        if remove_source {
            match trash::delete(&source) {
                Ok(()) => {
                    source_removed = true;
                    moved_ids.push(item.id);
                    changed_pods.insert(item.pod_id);
                }
                Err(error) => result
                    .warnings
                    .push(issue(format!("已复制，但源文件无法移入回收站: {error}"))),
            }
        }
        successful_targets.push((
            item.id,
            source.clone(),
            resolved_target.clone(),
            source_removed,
        ));
    }

    if !moved_ids.is_empty() {
        let delete = (|| -> Result<(), String> {
            let mut connection = state.db.lock().unwrap();
            let transaction = connection
                .transaction()
                .map_err(|error| error.to_string())?;
            db::delete_items_by_ids(&transaction, &moved_ids)?;
            transaction.commit().map_err(|error| error.to_string())
        })();
        match delete {
            Ok(()) => state.mark_staged(),
            Err(error) => {
                for id in &moved_ids {
                    let name = items
                        .iter()
                        .find(|item| item.id == *id)
                        .map(|item| item.name.clone())
                        .unwrap_or_else(|| format!("条目 {id}"));
                    result.warnings.push(ExportIssue {
                        id: *id,
                        name,
                        error: format!("文件已移动，但索引清理失败，将由 watcher 重试: {error}"),
                    });
                }
                state
                    .watcher_dirty
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    for pod_id in changed_pods {
        events::emit_items_changed(&app, pod_id as u64);
    }
    let failed_ids = result
        .failed
        .iter()
        .map(|issue| issue.id)
        .collect::<Vec<_>>();
    let operation_items = items
        .iter()
        .filter_map(|item| {
            if let Some((_, source, target, source_removed)) = successful_targets
                .iter()
                .find(|(item_id, _, _, _)| *item_id == item.id)
            {
                let compensation = if *source_removed {
                    Some(CompensationDraft {
                        kind: "restore_export_move".into(),
                        source_path: Some(target.to_string_lossy().to_string()),
                        target_path: Some(source.to_string_lossy().to_string()),
                        expected_signature: operations::signature(target).ok(),
                    })
                } else if conflict_strategy != "overwrite" {
                    Some(CompensationDraft {
                        kind: "delete_export_copy".into(),
                        source_path: None,
                        target_path: Some(target.to_string_lossy().to_string()),
                        expected_signature: operations::signature(target).ok(),
                    })
                } else {
                    None
                };
                return Some(OperationItemDraft {
                    item_id: Some(item.id),
                    name: item.name.clone(),
                    source_path: Some(source.to_string_lossy().to_string()),
                    target_path: Some(target.to_string_lossy().to_string()),
                    action: if *source_removed { "move" } else { "copy" }.into(),
                    status: "completed".into(),
                    error: (conflict_strategy == "overwrite")
                        .then(|| "覆盖操作无法恢复目标位置原有内容".into()),
                    snapshot: operations::snapshot(item),
                    compensation,
                });
            }
            if let Some(issue) = result.failed.iter().find(|issue| issue.id == item.id) {
                return Some(OperationItemDraft {
                    item_id: Some(item.id),
                    name: item.name.clone(),
                    source_path: Some(item.staging_path.clone()),
                    target_path: Some(destination.join(&item.name).to_string_lossy().to_string()),
                    action: mode.clone(),
                    status: "failed".into(),
                    error: Some(issue.error.clone()),
                    snapshot: operations::snapshot(item),
                    compensation: None,
                });
            }
            if result.skipped_ids.contains(&item.id) || result.stale_ids.contains(&item.id) {
                return Some(OperationItemDraft {
                    item_id: Some(item.id),
                    name: item.name.clone(),
                    source_path: Some(item.staging_path.clone()),
                    target_path: Some(destination.join(&item.name).to_string_lossy().to_string()),
                    action: mode.clone(),
                    status: if result.stale_ids.contains(&item.id) {
                        "stale"
                    } else {
                        "skipped"
                    }
                    .into(),
                    error: None,
                    snapshot: operations::snapshot(item),
                    compensation: None,
                });
            }
            None
        })
        .collect::<Vec<_>>();
    if !operation_items.is_empty() {
        let has_compensation = operation_items
            .iter()
            .any(|item| item.compensation.is_some());
        let retry = (!failed_ids.is_empty()).then(|| {
            serde_json::json!({
                "ids": failed_ids,
                "destination": destination,
                "mode": mode,
            })
        });
        let history = OperationDraft {
            kind: "export".into(),
            pod_id: items.first().map(|item| item.pod_id),
            summary: format!(
                "{} {} 项到 {}",
                if mode == "move" { "移动" } else { "复制" },
                result.completed_ids.len(),
                destination.display()
            ),
            status: if result.failed.is_empty() && result.warnings.is_empty() {
                "completed".into()
            } else {
                "partial".into()
            },
            undoable_until: has_compensation
                .then(|| db::now_ms().saturating_add(operations::BASIC_UNDO_MS)),
            metadata: retry
                .map(|retry| serde_json::json!({ "retry": retry }))
                .unwrap_or_else(|| serde_json::json!({})),
            items: operation_items,
        };
        if let Err(error) = operations::record(&state.db.lock().unwrap(), history) {
            crate::logging::write(&format!("[operations] 记录导出操作失败: {error}"));
        }
    }
    Ok(result)
}
