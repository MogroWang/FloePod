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
    // 只解析父目录；删除前解析符号链接本身会误删它指向的目标。
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

    // 旧索引仍占用文件名，避免先创建成功、再因 UNIQUE 入库失败而回删。
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
                        let record = file_ops::move_into_staging(source, &target)
                            .map_err(|error| format!("移动 {name} 失败: {error}"))?;
                        // 隔离件在入库前登记台账：崩溃后才能区分"移动已提交"与"未提交"。
                        if let Some(quarantine) = record.quarantine.as_ref() {
                            let resolved_target = settings::resolve_path(&target)?;
                            let connection = state.db.lock().unwrap();
                            db::insert_pending_move(
                                &connection,
                                &db::PendingMove {
                                    quarantine_path: quarantine.to_string_lossy().to_string(),
                                    original_path: record.original.to_string_lossy().to_string(),
                                    target_path: resolved_target.to_string_lossy().to_string(),
                                },
                            )?;
                        }
                        moves.push(record);
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
        drop_pending_moves(&state, &moves);
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
            drop_pending_moves(&state, &moves);
            return Err(with_rollback(error, rollback));
        }
    };

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

/// 按 id 打开一个暂存条目：路径必须通过 item_path 校验（属于某匣的暂存目录、
/// 非 reparse 点），不能让 WebView 直接驱使系统打开任意路径。
pub fn open_staged_item(app: &AppHandle, item_id: i64) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let state = app.state::<AppState>();
    let (current, item) = {
        let connection = state.db.lock().unwrap();
        let item = db::items_by_ids(&connection, &[item_id])?
            .into_iter()
            .next()
            .ok_or_else(|| "条目不存在".to_string())?;
        (load_settings_from(&connection, &state)?, item)
    };
    settings::validate_pod_for_io(&current, &data_dir(&state), item.pod_id as u64)?;
    let path = item_path(&item, &current)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|error| format!("打开「{}」失败: {error}", item.name))
}

/// 打开匣的暂存文件夹（设置页入口）。文件夹经 resolve_path 解析并校验归属。
pub fn open_pod_folder(app: &AppHandle, pod_id: u64) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let state = app.state::<AppState>();
    let current = validated_settings(&state, pod_id)?;
    let pod = current
        .pods
        .iter()
        .find(|pod| pod.id == pod_id)
        .ok_or_else(|| "匣不存在".to_string())?;
    let path = settings::resolve_path(Path::new(&pod.staging_folder))?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|error| format!("打开文件夹失败: {error}"))
}

/// Windows 保留设备名：作为文件名主干时（如 `CON.txt`）会被 Win32 解析到设备，
/// 导致 `create_new` 以费解的错误失败，这里统一加 `_` 前缀规避。
fn is_reserved_device_stem(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
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
    } else if is_reserved_device_stem(trimmed) {
        format!("_{trimmed}")
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

/// 把隔离件恢复为源文件。原路径被占用时在原目录内起一个不冲突的名字。
fn restore_quarantine(record: &db::PendingMove) -> Result<PathBuf, String> {
    let quarantine = PathBuf::from(&record.quarantine_path);
    let original = PathBuf::from(&record.original_path);
    let parent = original
        .parent()
        .ok_or_else(|| format!("恢复路径没有父目录: {}", original.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("无法重建源目录 {}: {error}", parent.display()))?;
    let name = original
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .ok_or_else(|| format!("恢复路径无效: {}", original.display()))?;
    let mut reserved = HashSet::new();
    let destination = match fs::symlink_metadata(&original) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => original,
        Ok(_) => file_ops::unique_target(parent, &name, &mut reserved)?,
        Err(error) => return Err(format!("无法检查原路径 {}: {error}", original.display())),
    };
    fs::rename(&quarantine, &destination).map_err(|error| {
        format!(
            "无法把隔离件 {} 恢复为 {}: {error}",
            quarantine.display(),
            destination.display()
        )
    })?;
    Ok(destination)
}

/// 启动清扫入口：按台账恢复被跨盘移动中断的源文件。
pub fn recover_pending_moves(app: &AppHandle) {
    recover_pending_moves_state(&app.state::<AppState>());
}

/// 移动已提交 -> 隔离件只是残留副本，删除；移动未提交 -> 隔离件是唯一原件，恢复。
/// 恢复 / 删除失败时保留台账行，下次启动重试。
fn recover_pending_moves_state(state: &AppState) {
    let _operation = state.file_ops.lock().unwrap();
    let records = {
        let connection = state.db.lock().unwrap();
        match db::list_pending_moves(&connection) {
            Ok(records) => records,
            Err(error) => {
                crate::logging::write(&format!("[recovery] 读取跨盘移动台账失败: {error}"));
                return;
            }
        }
    };
    for record in records {
        let quarantine = PathBuf::from(&record.quarantine_path);
        match fs::symlink_metadata(&quarantine) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let connection = state.db.lock().unwrap();
                let _ = db::delete_pending_move(&connection, &record.quarantine_path);
            }
            Err(error) => {
                crate::logging::write(&format!(
                    "[recovery] 无法检查隔离件 {}: {error}",
                    quarantine.display()
                ));
                continue;
            }
            Ok(_) => {}
        }
        let committed = {
            let connection = state.db.lock().unwrap();
            db::find_by_path(&connection, &record.target_path)
                .map(|found| found.is_some())
                .unwrap_or(false)
        };
        let outcome = if committed {
            file_ops::remove_path(&quarantine).map(|_| "已清理已提交移动的隔离副本".to_string())
        } else {
            restore_quarantine(&record)
                .map(|restored| format!("已恢复被中断移动的源文件: {}", restored.display()))
        };
        match outcome {
            Ok(message) => {
                let connection = state.db.lock().unwrap();
                let _ = db::delete_pending_move(&connection, &record.quarantine_path);
                crate::logging::write(&format!("[recovery] {message}"));
            }
            Err(error) => {
                crate::logging::write(&format!(
                    "[recovery] 处理隔离件 {} 失败（保留台账，下次启动重试）: {error}",
                    quarantine.display()
                ));
            }
        }
    }
    scan_legacy_stranded(state);
}

/// 历史版本（1.0.0 及更早）没有台账，遗留在暂存目录里的内部临时文件
/// 无法可靠还原（原名已丢失），只留痕提醒用户手动处理。
fn scan_legacy_stranded(state: &AppState) {
    let Ok(current) = load_settings(state) else {
        return;
    };
    for pod in current.pods.iter().filter(|pod| pod.enabled) {
        let Ok(root) = settings::resolve_path(Path::new(&pod.staging_folder)) else {
            continue;
        };
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if file_ops::is_internal_temp_name(&name.to_string_lossy()) {
                crate::logging::write(&format!(
                    "[recovery] 发现旧版本遗留的内部临时文件（含未还原数据，请手动确认）: {}",
                    entry.path().display()
                ));
            }
        }
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

    #[test]
    fn reserved_device_names_are_prefixed_for_win32() {
        assert_eq!(sanitize_text_name("CON"), "_CON");
        assert_eq!(text_file_base(Some("nul"), "正文"), "_nul");
        assert_eq!(text_file_base(Some("com1"), "正文"), "_com1");
        // 只有“点号前的主干”完全等于保留名才需要规避；
        // 常规名称、只是包含保留词的名字不受影响
        assert_eq!(sanitize_text_name("console"), "console");
        assert_eq!(sanitize_text_name("COM1 调试"), "COM1 调试");
        assert_eq!(sanitize_text_name("我的 CON 记录"), "我的 CON 记录");
    }

    #[test]
    fn interrupted_cross_volume_moves_recover_from_ledger() {
        let temporary = tempfile::tempdir().unwrap();
        let pod_root = temporary.path().join("stage");
        fs::create_dir_all(&pod_root).unwrap();
        let source_dir = temporary.path().join("src");
        fs::create_dir_all(&source_dir).unwrap();
        let original = source_dir.join("报告.docx");
        fs::write(&original, b"payload").unwrap();

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        let quarantine = source_dir.join(".floepod-move-source-1-0000000000000001");
        fs::rename(&original, &quarantine).unwrap();
        db::insert_pending_move(
            &conn,
            &db::PendingMove {
                quarantine_path: quarantine.to_string_lossy().to_string(),
                original_path: original.to_string_lossy().to_string(),
                target_path: pod_root.join("报告.docx").to_string_lossy().to_string(),
            },
        )
        .unwrap();

        // 目标未入库 -> 未提交 -> 恢复原文件
        let state = AppState::new(conn, temporary.path().join("data"));
        recover_pending_moves_state(&state);
        assert!(original.is_file());
        assert!(!quarantine.exists());
        assert!(load_settings(&state).is_ok());
    }

    #[test]
    fn committed_moves_drop_the_quarantine_copy_on_recovery() {
        let temporary = tempfile::tempdir().unwrap();
        let source_dir = temporary.path().join("src");
        fs::create_dir_all(&source_dir).unwrap();
        let quarantine = source_dir.join(".floepod-move-source-1-0000000000000002");
        fs::write(&quarantine, b"stale copy").unwrap();

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        let target = quarantine
            .with_extension("staged")
            .to_string_lossy()
            .to_string();
        db::insert_item(
            &conn,
            &StagedItem {
                id: 0,
                pod_id: 1,
                kind: "file".into(),
                staging_path: target.clone(),
                original_path: None,
                name: "a.staged".into(),
                ext: Some("staged".into()),
                size: 0,
                created_at: db::now_ms(),
            },
        )
        .unwrap();
        db::insert_pending_move(
            &conn,
            &db::PendingMove {
                quarantine_path: quarantine.to_string_lossy().to_string(),
                original_path: source_dir.join("a.bin").to_string_lossy().to_string(),
                target_path: target,
            },
        )
        .unwrap();

        // 目标已在库 -> 已提交 -> 删除残留副本
        let state = AppState::new(conn, temporary.path().join("data"));
        recover_pending_moves_state(&state);
        assert!(!quarantine.exists());
        let connection = state.db.lock().unwrap();
        assert!(db::list_pending_moves(&connection).unwrap().is_empty());
    }
}
