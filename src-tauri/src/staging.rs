use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db::{self, StagedItem};
use crate::events;
use crate::file_ops::{self, StagedMove};
use crate::lnk;
use crate::settings::{self, Pod, Settings};
use crate::state::AppState;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn data_dir(state: &AppState) -> String {
    state.data_dir.to_string_lossy().to_string()
}

pub fn load_settings(state: &AppState) -> Result<Settings, String> {
    let connection = state.db.lock().unwrap();
    load_settings_from(&connection, state)
}

pub fn load_settings_from(
    connection: &rusqlite::Connection,
    state: &AppState,
) -> Result<Settings, String> {
    settings::load(connection, &data_dir(state), VERSION)
}

pub fn validated_settings(state: &AppState, pod_id: u64) -> Result<Settings, String> {
    let current = load_settings(state)?;
    settings::validate(&current, &data_dir(state))?;
    settings::validate_pod_for_io(&current, &data_dir(state), pod_id)?;
    Ok(current)
}

pub fn validate_item_pods(
    current: &Settings,
    state: &AppState,
    items: &[StagedItem],
) -> Result<(), String> {
    let pod_ids: HashSet<u64> = items.iter().map(|item| item.pod_id as u64).collect();
    for pod_id in pod_ids {
        settings::validate_pod_for_io(current, &data_dir(state), pod_id)?;
    }
    Ok(())
}

pub fn item_path(item: &StagedItem, current: &Settings) -> Result<PathBuf, String> {
    let pod = current
        .pods
        .iter()
        .find(|pod| pod.id == item.pod_id as u64)
        .ok_or_else(|| format!("条目「{}」所属的匣已不存在", item.name))?;
    let root = settings::resolve_path(Path::new(&pod.staging_folder))?;
    let raw = PathBuf::from(&item.staging_path);
    let name = raw
        .file_name()
        .ok_or_else(|| format!("条目「{}」的路径无效", item.name))?;
    let parent = raw
        .parent()
        .ok_or_else(|| format!("条目「{}」的路径无效", item.name))?;
    // Resolve the parent first. Canonicalizing a symlink leaf before deletion
    // would turn a request to remove the link into a request to remove its target.
    let safe_path = settings::resolve_path(parent)?.join(name);
    if !settings::path_is_within(&safe_path, &root) || settings::paths_equal(&safe_path, &root) {
        return Err(format!("条目「{}」已不在当前匣的暂存目录内", item.name));
    }
    match fs::symlink_metadata(&raw) {
        Ok(metadata) if file_ops::is_reparse_or_symlink(&metadata) => {
            return Err(format!("条目「{}」是符号链接或目录重解析点", item.name));
        }
        Ok(_) => {
            let resolved = settings::resolve_path(&raw)?;
            if !settings::paths_equal(&resolved, &safe_path) {
                return Err(format!("条目「{}」的路径解析结果不一致", item.name));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("无法检查条目「{}」: {error}", item.name)),
    }
    Ok(safe_path)
}

fn indexed_paths(state: &AppState, pod_id: u64) -> Result<HashSet<String>, String> {
    let connection = state.db.lock().unwrap();
    Ok(db::items_of_pod(&connection, pod_id as i64)?
        .into_iter()
        .map(|item| settings::path_key(Path::new(&item.staging_path)))
        .collect())
}

fn staging_directory(pod: &Pod) -> Result<PathBuf, String> {
    let directory = PathBuf::from(&pod.staging_folder);
    fs::create_dir_all(&directory).map_err(|error| format!("暂存文件夹不可用: {error}"))?;
    Ok(directory)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageWarning {
    name: String,
    error: String,
}

#[derive(Debug, Default, Serialize)]
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
    let resolved_directory = settings::resolve_path(&directory)?;
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

    // A stale index still reserves its name. Otherwise the file can be created
    // successfully only for the later UNIQUE insert to fail and remove it again.
    let mut reserved = indexed_paths(&state, pod_id)?;
    let mut drafts = Vec::new();
    let mut created_paths = Vec::new();
    let mut moves: Vec<StagedMove> = Vec::new();

    let prepare = (|| -> Result<(), String> {
        match action.as_str() {
            "shortcut" => {
                let mut pairs = Vec::new();
                for source in &sources {
                    let name = source
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| "目标".into());
                    let target = file_ops::unique_target(
                        &directory,
                        &lnk::shortcut_name_for(&name),
                        &mut reserved,
                    )?;
                    pairs.push((source.clone(), target));
                }
                if let Err(error) = lnk::create_shortcuts(&pairs) {
                    for (_, target) in &pairs {
                        let _ = file_ops::remove_path(target);
                    }
                    return Err(error);
                }
                created_paths.extend(pairs.iter().map(|(_, target)| target.clone()));
                for (source, target) in pairs {
                    if fs::symlink_metadata(&target).is_err() {
                        return Err(format!("快捷方式未生成: {}", target.display()));
                    }
                    let target = settings::resolve_path(&target)?;
                    if !settings::path_is_within(&target, &resolved_directory) {
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
                            settings::resolve_path(&source)?
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
                for source in &sources {
                    let metadata = fs::symlink_metadata(source)
                        .map_err(|error| format!("无法读取 {}: {error}", source.display()))?;
                    if file_ops::is_reparse_or_symlink(&metadata) {
                        return Err(format!(
                            "暂不支持符号链接或目录重解析点: {}",
                            source.display()
                        ));
                    }
                    let resolved_source = settings::resolve_path(source)?;
                    if resolved_source.parent().is_none() {
                        return Err(format!("不能暂存文件系统根目录: {}", source.display()));
                    }
                    let is_directory = metadata.is_dir();
                    let original_path = resolved_source.to_string_lossy().to_string();
                    let name = source
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| "未命名".into());
                    let target = file_ops::unique_target(&directory, &name, &mut reserved)?;
                    file_ops::ensure_distinct_target(source, &target)?;
                    if operation == "move" {
                        moves.push(
                            file_ops::move_into_staging(source, &target)
                                .map_err(|error| format!("移动 {name} 失败: {error}"))?,
                        );
                    } else if let Err(error) = file_ops::copy_path(source, &target) {
                        let _ = file_ops::remove_path(&target);
                        return Err(format!("复制 {name} 失败: {error}"));
                    }
                    created_paths.push(target.clone());
                    let target = settings::resolve_path(&target)?;
                    if !settings::path_is_within(&target, &resolved_directory) {
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
                        ext: file_ops::extension(&name),
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
        return Err(with_rollback(error, rollback));
    }

    let persist = (|| -> Result<Vec<StagedItem>, String> {
        let mut connection = state.db.lock().unwrap();
        let current = load_settings_from(&connection, &state)?;
        settings::validate_pod_for_io(&current, &data_dir(&state), pod.id)?;
        let current_pod = current
            .pods
            .iter()
            .find(|candidate| candidate.id == pod.id)
            .ok_or_else(|| "暂存过程中匣已被删除".to_string())?;
        let current_directory = settings::resolve_path(Path::new(&current_pod.staging_folder))?;
        if !settings::paths_equal(&current_directory, &resolved_directory) {
            return Err("暂存过程中匣的文件夹已改变".into());
        }
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let mut saved = Vec::with_capacity(drafts.len());
        for draft in &drafts {
            saved.push(db::insert_item(&transaction, draft)?);
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(saved)
    })();
    let items = match persist {
        Ok(items) => items,
        Err(error) => {
            let rollback = if action == "move" {
                file_ops::rollback_staged_moves(&moves)
            } else {
                created_paths
                    .iter()
                    .rev()
                    .filter_map(|path| {
                        file_ops::remove_path(path)
                            .err()
                            .map(|error| format!("{}: {error}", path.display()))
                    })
                    .collect()
            };
            return Err(with_rollback(error, rollback));
        }
    };

    let mut warnings = Vec::new();
    for record in &moves {
        let Some(quarantine) = record.quarantine.as_ref() else {
            continue;
        };
        if let Err(error) = file_ops::remove_path(quarantine) {
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
    state.mark_staged();
    events::emit_items_changed(&app, pod.id);
    Ok(StagePathsResult { items, warnings })
}

fn with_rollback(error: String, rollback: Vec<String>) -> String {
    if rollback.is_empty() {
        error
    } else {
        format!("{error}；回滚未完全成功：{}", rollback.join("；"))
    }
}

fn sanitize_text_name(raw: &str) -> String {
    let invalid = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let cleaned: String = raw
        .chars()
        .take(48)
        .map(|character| {
            if invalid.contains(&character) || character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "文字".to_string()
    } else {
        trimmed.to_string()
    }
}

fn text_file_base(title: Option<&str>, content: &str) -> String {
    let raw = title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| content.lines().next().unwrap_or("文字"));
    let without_extension = if raw.to_ascii_lowercase().ends_with(".txt") {
        &raw[..raw.len().saturating_sub(4)]
    } else {
        raw
    };
    sanitize_text_name(without_extension)
}

pub fn stage_text(
    app: AppHandle,
    pod_id: u64,
    content: String,
    title: Option<String>,
) -> Result<StagedItem, String> {
    if content.trim().is_empty() {
        return Err("内容为空".into());
    }
    let state = app.state::<AppState>();
    let _operation = state.file_ops.lock().unwrap();
    let current = validated_settings(&state, pod_id)?;
    let pod = current
        .pods
        .iter()
        .find(|pod| pod.id == pod_id)
        .cloned()
        .ok_or_else(|| "匣不存在".to_string())?;
    let directory = staging_directory(&pod)?;
    let resolved_directory = settings::resolve_path(&directory)?;
    let base = text_file_base(title.as_deref(), &content);
    let mut reserved = indexed_paths(&state, pod_id)?;
    let target = file_ops::unique_target(&directory, &format!("{base}.txt"), &mut reserved)?;
    let size = content.len() as i64;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|error| format!("创建文字文件失败: {error}"))?;
    if let Err(error) = std::io::Write::write_all(&mut output, content.as_bytes()) {
        drop(output);
        let cleanup = file_ops::remove_path(&target).err();
        return Err(match cleanup {
            Some(cleanup) => format!("写入失败: {error}；清理半成品失败: {cleanup}"),
            None => format!("写入失败: {error}"),
        });
    }
    drop(output);
    let target = match settings::resolve_path(&target) {
        Ok(target) => target,
        Err(error) => {
            let cleanup = file_ops::remove_path(&target).err();
            return Err(match cleanup {
                Some(cleanup) => format!("{error}；清理未入库文字文件失败: {cleanup}"),
                None => error,
            });
        }
    };
    if !settings::path_is_within(&target, &resolved_directory) {
        let cleanup = file_ops::remove_path(&target).err();
        return Err(match cleanup {
            Some(cleanup) => format!("文字暂存目标越出暂存文件夹；清理失败: {cleanup}"),
            None => "文字暂存目标越出暂存文件夹".into(),
        });
    }

    let draft = StagedItem {
        id: 0,
        pod_id: pod.id as i64,
        kind: "text".into(),
        staging_path: target.to_string_lossy().to_string(),
        original_path: None,
        name: target
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
        ext: Some("txt".into()),
        size,
        created_at: db::now_ms(),
    };
    let persist = (|| -> Result<StagedItem, String> {
        let connection = state.db.lock().unwrap();
        let current = load_settings_from(&connection, &state)?;
        settings::validate_pod_for_io(&current, &data_dir(&state), pod.id)?;
        let current_pod = current
            .pods
            .iter()
            .find(|candidate| candidate.id == pod.id)
            .ok_or_else(|| "文字写入过程中匣已被删除".to_string())?;
        let current_directory = settings::resolve_path(Path::new(&current_pod.staging_folder))?;
        if !settings::paths_equal(&current_directory, &resolved_directory) {
            return Err("文字写入过程中匣的文件夹已改变".into());
        }
        db::insert_item(&connection, &draft)
    })();
    let item = match persist {
        Ok(item) => item,
        Err(error) => {
            let cleanup = file_ops::remove_path(&target).err();
            return Err(match cleanup {
                Some(cleanup) => format!("{error}；清理未入库文字文件失败: {cleanup}"),
                None => error,
            });
        }
    };
    state.mark_staged();
    events::emit_items_changed(&app, pod.id);
    Ok(item)
}

pub fn list_pod_items(app: &AppHandle, pod_id: u64) -> Result<Vec<StagedItem>, String> {
    let state = app.state::<AppState>();
    let connection = state.db.lock().unwrap();
    db::items_of_pod(&connection, pod_id as i64)
}

pub fn remove_items(app: AppHandle, ids: Vec<i64>, delete_files: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _operation = state.file_ops.lock().unwrap();
    let (current, items) = {
        let connection = state.db.lock().unwrap();
        (
            load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, &ids)?,
        )
    };
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
    if delete_files {
        for (item, path) in validated {
            match fs::symlink_metadata(&path) {
                Ok(_) => match trash::delete(&path) {
                    Ok(()) => removed_ids.push(item.id),
                    Err(error) => failed.push(format!("{}: {error}", item.name)),
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => removed_ids.push(item.id),
                Err(error) => failed.push(format!("{}: {error}", item.name)),
            }
        }
    } else {
        removed_ids.extend(items.iter().map(|item| item.id));
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
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("部分文件无法移入回收站：{}", failed.join("；")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_names_keep_the_readable_head_and_one_txt_suffix() {
        assert_eq!(sanitize_text_name("héllo world"), "héllo world");
        assert_eq!(sanitize_text_name("a<b>c:d"), "a b c d");
        assert_eq!(sanitize_text_name("   "), "文字");
        assert_eq!(sanitize_text_name(&"字".repeat(80)).chars().count(), 48);
        assert_eq!(text_file_base(Some("实验记录"), "正文"), "实验记录");
        assert_eq!(text_file_base(Some("实验记录.txt"), "正文"), "实验记录");
        assert_eq!(text_file_base(Some("  "), "第一行\n第二行"), "第一行");
    }
}
