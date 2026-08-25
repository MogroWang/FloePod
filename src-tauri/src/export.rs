use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db;
use crate::events;
use crate::file_ops;
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
                    // The stale row is removed, but no destination file was produced.
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
        if mode == "move" {
            match trash::delete(&source) {
                Ok(()) => {
                    moved_ids.push(item.id);
                    changed_pods.insert(item.pod_id);
                }
                Err(error) => result
                    .warnings
                    .push(issue(format!("已复制，但源文件无法移入回收站: {error}"))),
            }
        }
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
    Ok(result)
}
