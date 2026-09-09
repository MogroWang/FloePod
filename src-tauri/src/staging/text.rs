use std::fs;
use std::path::Path;

use tauri::{AppHandle, Manager};

use crate::db::{self, StagedItem};
use crate::events;
use crate::file_ops::{self};
use crate::operations::{self, CompensationDraft, OperationDraft, OperationItemDraft};
use crate::security;
use crate::settings::{self};
use crate::state::AppState;

use super::context::*;
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

pub(super) fn sanitize_text_name(raw: &str) -> String {
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

pub(super) fn text_file_base(title: Option<&str>, content: &str) -> String {
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
    security::require_unlocked(&app, pod_id)?;
    let _permit = state.tasks.enter()?;
    let _operation = state.file_ops.lock().unwrap();
    let current = validated_settings(&state, pod_id)?;
    let pod = current
        .pods
        .iter()
        .find(|pod| pod.id == pod_id)
        .cloned()
        .ok_or_else(|| "匣不存在".to_string())?;
    let directory = staging_directory(&pod)?;
    let resolved_directory = crate::file_paths::resolve_path(&directory)?;
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
    let target = match crate::file_paths::resolve_path(&target) {
        Ok(target) => target,
        Err(error) => {
            let cleanup = file_ops::remove_path(&target).err();
            return Err(match cleanup {
                Some(cleanup) => format!("{error}；清理未入库文字文件失败: {cleanup}"),
                None => error,
            });
        }
    };
    if !crate::file_paths::path_is_within(&target, &resolved_directory) {
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
        let current_directory =
            crate::file_paths::resolve_path(Path::new(&current_pod.staging_folder))?;
        if !crate::file_paths::paths_equal(&current_directory, &resolved_directory) {
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
    let operation_item = OperationItemDraft {
        item_id: Some(item.id),
        name: item.name.clone(),
        source_path: None,
        target_path: Some(item.staging_path.clone()),
        action: "text".into(),
        status: "completed".into(),
        error: None,
        snapshot: operations::snapshot(&item),
        compensation: Some(CompensationDraft {
            kind: "delete_staged_copy".into(),
            source_path: None,
            target_path: Some(item.staging_path.clone()),
            expected_signature: operations::signature(&target).ok(),
        }),
    };
    if let Err(error) = operations::record(
        &state.db.lock().unwrap(),
        OperationDraft::completed(
            "stage_text",
            Some(pod.id as i64),
            format!("暂存文字「{}」到「{}」", item.name, pod.name),
            serde_json::json!({}),
            vec![operation_item],
        ),
    ) {
        crate::logging::write(&format!("[operations] 记录文字暂存失败: {error}"));
    }
    state.mark_staged();
    events::emit_items_changed(&app, pod.id);
    Ok(item)
}
