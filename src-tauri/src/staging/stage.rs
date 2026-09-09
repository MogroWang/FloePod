use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db::{self, StagedItem};
use crate::events;
use crate::file_ops::{self, StagedMove};
use crate::lnk;
use crate::operations::{self, CompensationDraft, OperationDraft, OperationItemDraft};
use crate::policy;
use crate::rules;
use crate::security;
use crate::settings::{self, Pod};
use crate::state::AppState;

use super::context::*;
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StageWarning {
    name: String,
    error: String,
}

#[derive(Debug, Default, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StagePathsResult {
    items: Vec<StagedItem>,
    warnings: Vec<StageWarning>,
}

pub fn stage_paths(
    app: AppHandle,
    pod_id: u64,
    paths: Vec<String>,
    action: String,
) -> Result<StagePathsResult, String> {
    let state = app.state::<AppState>();
    policy::enforce_stage(&action)?;
    security::require_unlocked(&app, pod_id)?;
    let _permit = state.tasks.enter()?;
    let _operation = state.file_ops.lock().unwrap();
    if !matches!(action.as_str(), "copy" | "move" | "shortcut") {
        return Err(format!("未知动作: {action}"));
    }
    let current = validated_settings(&state, pod_id)?;
    let pod = current
        .pods
        .iter()
        .find(|pod| pod.id == pod_id)
        .cloned()
        .ok_or_else(|| "匣不存在".to_string())?;
    let directory = staging_directory(&pod)?;
    let resolved_directory = crate::file_paths::resolve_path(&directory)?;
    if paths.is_empty() {
        return Err("没有可暂存的文件".into());
    }

    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let source = PathBuf::from(path);
        if !source.is_absolute() {
            return Err(format!("源路径必须是绝对路径: {}", source.display()));
        }
        fs::symlink_metadata(&source)
            .map_err(|error| format!("无法读取源路径 {}: {error}", source.display()))?;
        sources.push(source);
    }

    let prepared = prepare_files(
        &state,
        &pod,
        &sources,
        &action,
        &directory,
        &resolved_directory,
    )?;
    let items = commit_or_rollback(&state, &pod, &action, &resolved_directory, &prepared)?;
    finish_committed(&app, &state, &pod, &action, items, prepared.moves)
}

struct PreparedBatch {
    drafts: Vec<StagedItem>,
    created_paths: Vec<PathBuf>,
    moves: Vec<StagedMove>,
}

fn prepare_files(
    state: &AppState,
    pod: &Pod,
    sources: &[PathBuf],
    action: &str,
    directory: &Path,
    resolved_directory: &Path,
) -> Result<PreparedBatch, String> {
    // 旧索引仍占用文件名，避免先创建成功、再因 UNIQUE 入库失败而回删。
    let mut reserved = indexed_paths(state, pod.id)?;
    let mut drafts = Vec::new();
    let mut created_paths = Vec::new();
    let mut moves: Vec<StagedMove> = Vec::new();

    let prepare = (|| -> Result<(), String> {
        match action {
            "shortcut" => {
                let mut pairs = Vec::new();
                for source in sources {
                    let name = source
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| "目标".into());
                    let target = file_ops::unique_target(
                        directory,
                        &lnk::shortcut_name_for(&name),
                        &mut reserved,
                    )?;
                    pairs.push((source.clone(), target));
                }
                lnk::create_shortcuts(&pairs)?;
                created_paths.extend(pairs.iter().map(|(_, target)| target.clone()));
                for (source, target) in pairs {
                    if fs::symlink_metadata(&target).is_err() {
                        return Err(format!("快捷方式未生成: {}", target.display()));
                    }
                    let target = crate::file_paths::resolve_path(&target)?;
                    if !crate::file_paths::path_is_within(&target, resolved_directory) {
                        return Err("快捷方式目标路径越出暂存文件夹".into());
                    }
                    let name = target
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default();
                    drafts.push(StagedItem {
                        id: 0,
                        pod_id: pod.id as i64,
                        kind: "shortcut".into(),
                        staging_path: target.to_string_lossy().to_string(),
                        original_path: Some(
                            crate::file_paths::resolve_path(&source)?
                                .to_string_lossy()
                                .to_string(),
                        ),
                        name,
                        ext: Some("lnk".into()),
                        size: 0,
                        created_at: db::now_ms(),
                    });
                }
            }
            operation @ ("copy" | "move") => {
                let mut duplicate_candidates =
                    if pod.rules.enabled && pod.rules.duplicate_policy == "reject" {
                        let connection = state.db.lock().unwrap();
                        db::items_of_pod(&connection, pod.id as i64)?
                            .into_iter()
                            .map(|item| PathBuf::from(item.staging_path))
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                for source in sources {
                    let metadata = fs::symlink_metadata(source)
                        .map_err(|error| format!("无法读取 {}: {error}", source.display()))?;
                    if file_ops::is_reparse_or_symlink(&metadata) {
                        return Err(format!(
                            "暂不支持符号链接或目录重解析点: {}",
                            source.display()
                        ));
                    }
                    rules::validate_source(pod, source, &metadata)?;
                    if pod.rules.enabled && pod.rules.duplicate_policy == "reject" {
                        if let Some(duplicate) = rules::duplicate_of(source, &duplicate_candidates)?
                        {
                            return Err(format!(
                                "规则拒绝「{}」：内容与 {} 重复",
                                source.display(),
                                duplicate.display()
                            ));
                        }
                    }
                    let resolved_source = crate::file_paths::resolve_path(source)?;
                    if resolved_source.parent().is_none() {
                        return Err(format!("不能暂存文件系统根目录: {}", source.display()));
                    }
                    let is_directory = metadata.is_dir();
                    let original_path = resolved_source.to_string_lossy().to_string();
                    let source_name = source
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| "未命名".into());
                    let planned = rules::target(directory, source, pod)?;
                    fs::create_dir_all(&planned.directory).map_err(|error| {
                        format!(
                            "无法创建规则子目录 {}: {error}",
                            planned.directory.display()
                        )
                    })?;
                    let target =
                        file_ops::unique_target(&planned.directory, &planned.name, &mut reserved)?;
                    file_ops::ensure_distinct_target(source, &target)?;
                    if operation == "move" {
                        let record = file_ops::move_into_staging(source, &target, |record| {
                            let quarantine =
                                record.quarantine.as_ref().ok_or("缺少移动恢复路径")?;
                            let resolved_target = crate::file_paths::resolve_path(&target)?;
                            db::insert_pending_move(
                                &state.db.lock().unwrap(),
                                &db::PendingMove {
                                    quarantine_path: quarantine.to_string_lossy().to_string(),
                                    original_path: record.original.to_string_lossy().to_string(),
                                    target_path: resolved_target.to_string_lossy().to_string(),
                                },
                            )
                        })
                        .map_err(|error| format!("移动 {source_name} 失败: {error}"))?;
                        moves.push(record);
                    } else if let Err(error) = file_ops::copy_path(source, &target) {
                        return Err(format!("复制 {source_name} 失败: {error}"));
                    }
                    created_paths.push(target.clone());
                    duplicate_candidates.push(target.clone());
                    let target = crate::file_paths::resolve_path(&target)?;
                    if !crate::file_paths::path_is_within(&target, resolved_directory) {
                        return Err("暂存目标路径越出暂存文件夹".into());
                    }
                    let size = if is_directory {
                        0
                    } else {
                        fs::metadata(&target)
                            .map(|meta| meta.len() as i64)
                            .unwrap_or(0)
                    };
                    drafts.push(StagedItem {
                        id: 0,
                        pod_id: pod.id as i64,
                        kind: if is_directory { "folder" } else { "file" }.into(),
                        staging_path: target.to_string_lossy().to_string(),
                        original_path: Some(original_path),
                        name: target
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        ext: file_ops::extension(&planned.name),
                        size,
                        created_at: db::now_ms(),
                    });
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    })();

    if let Err(error) = prepare {
        let rollback = if action == "move" {
            file_ops::rollback_staged_moves(&moves)
        } else {
            created_paths
                .iter()
                .filter_map(|path| {
                    file_ops::remove_path(path)
                        .err()
                        .map(|error| format!("{}: {error}", path.display()))
                })
                .collect()
        };
        drop_pending_moves(state, &moves);
        return Err(with_rollback(error, rollback));
    }

    Ok(PreparedBatch {
        drafts,
        created_paths,
        moves,
    })
}

fn commit_items(
    state: &AppState,
    pod: &Pod,
    resolved_directory: &Path,
    drafts: &[StagedItem],
) -> Result<Vec<StagedItem>, String> {
    let mut connection = state.db.lock().unwrap();
    let current = load_settings_from(&connection, state)?;
    settings::validate_pod_for_io(&current, &data_dir(state), pod.id)?;
    let current_pod = current
        .pods
        .iter()
        .find(|candidate| candidate.id == pod.id)
        .ok_or_else(|| "暂存过程中匣已被删除".to_string())?;
    let current_directory =
        crate::file_paths::resolve_path(Path::new(&current_pod.staging_folder))?;
    if !crate::file_paths::paths_equal(&current_directory, resolved_directory) {
        return Err("暂存过程中匣的文件夹已改变".into());
    }
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let mut saved = Vec::with_capacity(drafts.len());
    for draft in drafts {
        saved.push(db::insert_item(&transaction, draft)?);
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(saved)
}

fn commit_or_rollback(
    state: &AppState,
    pod: &Pod,
    action: &str,
    resolved_directory: &Path,
    prepared: &PreparedBatch,
) -> Result<Vec<StagedItem>, String> {
    let items = match commit_items(state, pod, resolved_directory, &prepared.drafts) {
        Ok(items) => items,
        Err(error) => {
            let rollback = if action == "move" {
                file_ops::rollback_staged_moves(&prepared.moves)
            } else {
                prepared
                    .created_paths
                    .iter()
                    .rev()
                    .filter_map(|path| {
                        file_ops::remove_path(path)
                            .err()
                            .map(|error| format!("{}: {error}", path.display()))
                    })
                    .collect()
            };
            drop_pending_moves(state, &prepared.moves);
            return Err(with_rollback(error, rollback));
        }
    };

    Ok(items)
}

fn finish_committed(
    app: &AppHandle,
    state: &AppState,
    pod: &Pod,
    action: &str,
    items: Vec<StagedItem>,
    moves: Vec<StagedMove>,
) -> Result<StagePathsResult, String> {
    let mut warnings = Vec::new();
    for record in &moves {
        let Some(quarantine) = record.quarantine.as_ref() else {
            continue;
        };
        match file_ops::remove_path(quarantine) {
            Ok(()) => {
                let connection = state.db.lock().unwrap();
                if let Err(error) =
                    db::delete_pending_move(&connection, &quarantine.to_string_lossy())
                {
                    crate::logging::write(&format!("[pending-move] 清理台账失败: {error}"));
                }
            }
            Err(error) => {
                let name = record
                    .original
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_else(|| record.original.display().to_string());
                warnings.push(StageWarning {
                    name,
                    error: format!(
                        "目标已暂存，但源卷临时副本 {} 清理失败: {error}",
                        quarantine.display()
                    ),
                });
            }
        }
    }
    let mut checksum_sidecars = Vec::new();
    if pod.rules.enabled && pod.rules.checksum_sidecar {
        for item in &items {
            let path = PathBuf::from(&item.staging_path);
            match rules::write_checksum_sidecar(&path) {
                Ok(Some(sidecar)) => checksum_sidecars.push(sidecar),
                Ok(None) => {}
                Err(error) => {
                    warnings.push(StageWarning {
                        name: item.name.clone(),
                        error: format!("文件已暂存，但生成 SHA-256 校验文件失败: {error}"),
                    });
                }
            }
        }
        state
            .watcher_dirty
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let mut operation_items = items
        .iter()
        .map(|item| {
            let target = PathBuf::from(&item.staging_path);
            OperationItemDraft {
                item_id: Some(item.id),
                name: item.name.clone(),
                source_path: item.original_path.clone(),
                target_path: Some(item.staging_path.clone()),
                action: action.to_string(),
                status: "completed".into(),
                error: None,
                snapshot: operations::snapshot(item),
                compensation: Some(CompensationDraft {
                    kind: if action == "move" {
                        "restore_stage_move".into()
                    } else {
                        "delete_staged_copy".into()
                    },
                    source_path: item.original_path.clone(),
                    target_path: Some(item.staging_path.clone()),
                    expected_signature: operations::signature(&target).ok(),
                }),
            }
        })
        .collect::<Vec<_>>();
    operation_items.extend(checksum_sidecars.into_iter().map(|sidecar| {
        OperationItemDraft {
            item_id: None,
            name: sidecar
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "SHA-256 校验文件".into()),
            source_path: None,
            target_path: Some(sidecar.to_string_lossy().to_string()),
            action: "checksum".into(),
            status: "completed".into(),
            error: None,
            snapshot: None,
            compensation: Some(CompensationDraft {
                kind: "delete_staged_copy".into(),
                source_path: None,
                target_path: Some(sidecar.to_string_lossy().to_string()),
                expected_signature: operations::signature(&sidecar).ok(),
            }),
        }
    }));
    let verb = match action {
        "move" => "移动",
        "shortcut" => "创建快捷方式",
        _ => "复制",
    };
    if let Err(error) = operations::record(
        &state.db.lock().unwrap(),
        OperationDraft::completed(
            "stage",
            Some(pod.id as i64),
            format!("{verb} {} 项到「{}」", items.len(), pod.name),
            serde_json::json!({}),
            operation_items,
        ),
    ) {
        crate::logging::write(&format!("[operations] 记录暂存操作失败: {error}"));
    }
    state.mark_staged();
    events::emit_items_changed(app, pod.id);
    Ok(StagePathsResult { items, warnings })
}

pub(super) fn with_rollback(error: String, rollback: Vec<String>) -> String {
    if rollback.is_empty() {
        error
    } else {
        format!("{error}；回滚未完全成功：{}", rollback.join("；"))
    }
}

/// 回滚后同步台账：隔离件已恢复（文件消失）才删除台账行；
/// 恢复失败的行保留，交给下次启动的 `recover_pending_moves` 重试。
fn drop_pending_moves(state: &AppState, moves: &[StagedMove]) {
    let connection = state.db.lock().unwrap();
    for record in moves {
        let Some(quarantine) = record.quarantine.as_ref() else {
            continue;
        };
        if fs::symlink_metadata(quarantine).is_ok() {
            continue;
        }
        if let Err(error) = db::delete_pending_move(&connection, &quarantine.to_string_lossy()) {
            crate::logging::write(&format!("[pending-move] 清理台账失败: {error}"));
        }
    }
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, AppState, Pod, PathBuf) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("stage");
        fs::create_dir(&root).unwrap();
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate(&connection).unwrap();
        let pod = Pod {
            id: 1,
            staging_folder: root.to_string_lossy().into(),
            ..Default::default()
        };
        let settings = settings::Settings {
            pods: vec![pod.clone()],
            ..Default::default()
        };
        settings::persist(&connection, &settings).unwrap();
        let state = AppState::new(connection, temporary.path().join("data"));
        let resolved = crate::file_paths::resolve_path(&root).unwrap();
        (temporary, state, pod, resolved)
    }
    #[test]
    fn prepare_failure_restores_every_preceding_move() {
        let (temporary, state, pod, root) = fixture();
        let source = temporary.path().join("source.txt");
        fs::write(&source, b"original").unwrap();
        let missing = temporary.path().join("missing.txt");
        assert!(prepare_files(
            &state,
            &pod,
            &[source.clone(), missing],
            "move",
            &root,
            &root
        )
        .is_err());
        assert_eq!(fs::read(source).unwrap(), b"original");
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
        assert!(db::items_of_pod(&state.db.lock().unwrap(), 1)
            .unwrap()
            .is_empty());
    }
    #[test]
    fn sqlite_failure_rolls_back_the_entire_copy_or_move_batch() {
        for action in ["copy", "move"] {
            let (temporary, state, pod, root) = fixture();
            let source = temporary.path().join("source.txt");
            fs::write(&source, b"original").unwrap();
            let mut prepared = prepare_files(
                &state,
                &pod,
                std::slice::from_ref(&source),
                action,
                &root,
                &root,
            )
            .unwrap();
            prepared.drafts.push(prepared.drafts[0].clone()); // Fail the second INSERT through the real UNIQUE constraint.
            assert!(commit_or_rollback(&state, &pod, action, &root, &prepared).is_err());
            assert_eq!(fs::read(&source).unwrap(), b"original");
            assert_eq!(fs::read_dir(root).unwrap().count(), 0);
            assert!(db::items_of_pod(&state.db.lock().unwrap(), 1)
                .unwrap()
                .is_empty());
        }
    }
    #[test]
    fn a_folder_change_before_commit_rolls_back_the_old_destination() {
        let (temporary, state, pod, root) = fixture();
        let source = temporary.path().join("source.txt");
        fs::write(&source, b"original").unwrap();
        let prepared = prepare_files(
            &state,
            &pod,
            std::slice::from_ref(&source),
            "move",
            &root,
            &root,
        )
        .unwrap();
        let replacement = temporary.path().join("replacement");
        fs::create_dir(&replacement).unwrap();
        let changed = Pod {
            staging_folder: replacement.to_string_lossy().into(),
            ..pod.clone()
        };
        settings::persist(
            &state.db.lock().unwrap(),
            &settings::Settings {
                pods: vec![changed],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(commit_or_rollback(&state, &pod, "move", &root, &prepared).is_err());
        assert_eq!(fs::read(source).unwrap(), b"original");
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
        assert_eq!(fs::read_dir(replacement).unwrap().count(), 0);
    }
}
