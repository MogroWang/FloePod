use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Emitter, Manager};

use crate::db::{self, StagedItem};
use crate::events;
use crate::manager;
use crate::settings::{self, Pod, Settings};
use crate::staging;
use crate::state::AppState;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn numeric(value: &serde_json::Value, field: &str) -> Result<f64, String> {
    let parsed = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
        .ok_or_else(|| format!("字段 {field} 必须是数字"))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(format!("字段 {field} 必须是有限数字"))
    }
}

fn unsigned(value: &serde_json::Value, field: &str) -> Result<u64, String> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
        .ok_or_else(|| format!("字段 {field} 必须是非负整数"))
}

fn string(value: &serde_json::Value, field: &str) -> Result<String, String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("字段 {field} 必须是字符串"))
}

fn apply_patch(pod: &mut Pod, patch: &serde_json::Value) -> Result<(), String> {
    let patch = patch
        .as_object()
        .ok_or_else(|| "匣配置补丁必须是对象".to_string())?;
    for (field, value) in patch {
        match field.as_str() {
            "name" => pod.name = string(value, field)?,
            "edge" => pod.edge = string(value, field)?,
            "monitor" => pod.monitor = string(value, field)?,
            "offset" => pod.offset = numeric(value, field)?,
            "stagingFolder" => pod.staging_folder = string(value, field)?,
            "opacity" => pod.opacity = numeric(value, field)?,
            "material" => pod.material = string(value, field)?,
            "panelWidth" => {
                pod.panel_width = u32::try_from(unsigned(value, field)?)
                    .map_err(|_| format!("字段 {field} 超出有效范围"))?;
            }
            "hoverDelayMs" => pod.hover_delay_ms = unsigned(value, field)?,
            "dropAction" => pod.drop_action = string(value, field)?,
            "enabled" => {
                pod.enabled = value
                    .as_bool()
                    .ok_or_else(|| format!("字段 {field} 必须是布尔值"))?;
            }
            _ => return Err(format!("未知匣配置字段: {field}")),
        }
    }
    Ok(())
}

fn from_config(config: &serde_json::Value) -> Result<Pod, String> {
    let mut pod = Pod::default();
    apply_patch(&mut pod, config)?;
    Ok(pod)
}

fn staging_folder_changed(old: &str, new: &str) -> Result<bool, String> {
    let old = old.trim();
    let new = new.trim();
    match (old.is_empty(), new.is_empty()) {
        (true, true) => Ok(false),
        (true, false) | (false, true) => Ok(true),
        (false, false) => Ok(!settings::configured_paths_equal(
            Path::new(old),
            Path::new(new),
        )?),
    }
}

pub fn create(
    app: AppHandle,
    config: serde_json::Value,
    reuse_existing: bool,
) -> Result<Pod, String> {
    let state = app.state::<AppState>();
    let _operation = state.settings_ops.lock().unwrap();
    let pod = {
        let connection = state.db.lock().unwrap();
        let mut pod = from_config(&config)?;
        let current = staging::load_settings_from(&connection, &state)?;
        let reusable = reuse_existing.then(|| {
            current.pods.iter().find(|existing| {
                !existing.staging_folder.is_empty()
                    && !pod.staging_folder.is_empty()
                    && matches!(
                        staging_folder_changed(&existing.staging_folder, &pod.staging_folder),
                        Ok(false)
                    )
            })
        });
        if let Some(existing) = reusable.flatten() {
            existing.clone()
        } else {
            pod.id = settings::next_pod_id(&connection, &staging::data_dir(&state), VERSION)?;
            settings::upsert_pod(&connection, &pod, &staging::data_dir(&state), VERSION)?;
            pod
        }
    };
    manager::apply_settings(&app, &manager::current_settings(&app));
    state
        .watcher_dirty
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = app.emit(events::PODS_CHANGED, ());
    Ok(pod)
}

pub fn update(app: AppHandle, pod_id: u64, patch: serde_json::Value) -> Result<Pod, String> {
    let state = app.state::<AppState>();
    let _settings_operation = state.settings_ops.lock().unwrap();
    let file_operation = state.file_ops.lock().unwrap();
    let (pod, folder_changed, needs_reconcile) = {
        let mut connection = state.db.lock().unwrap();
        let mut pod = staging::load_settings_from(&connection, &state)?
            .pods
            .into_iter()
            .find(|pod| pod.id == pod_id)
            .ok_or_else(|| "匣不存在".to_string())?;
        let old_folder = pod.staging_folder.clone();
        let old_enabled = pod.enabled;
        apply_patch(&mut pod, &patch)?;
        let folder_changed = staging_folder_changed(&old_folder, &pod.staging_folder)?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        settings::upsert_pod(&transaction, &pod, &staging::data_dir(&state), VERSION)?;
        if folder_changed {
            // 更换目录只切换索引根目录，不移动原文件。
            db::delete_items_by_pod(&transaction, pod_id as i64)?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        let needs_reconcile = folder_changed || (!old_enabled && pod.enabled);
        (pod, folder_changed, needs_reconcile)
    };
    drop(file_operation);
    manager::apply_settings(&app, &manager::current_settings(&app));
    if needs_reconcile {
        state
            .watcher_dirty
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if folder_changed {
        events::emit_items_changed(&app, pod_id);
    }
    let _ = app.emit(events::PODS_CHANGED, ());
    Ok(pod)
}

pub fn delete(app: AppHandle, pod_id: u64, recycle_files: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _settings_operation = state.settings_ops.lock().unwrap();
    let file_operation = state.file_ops.lock().unwrap();
    let (current, items): (Settings, Vec<StagedItem>) = {
        let connection = state.db.lock().unwrap();
        (
            staging::load_settings_from(&connection, &state)?,
            db::items_of_pod(&connection, pod_id as i64)?,
        )
    };

    if recycle_files {
        settings::validate(&current, &staging::data_dir(&state))?;
        settings::validate_pod_for_io(&current, &staging::data_dir(&state), pod_id)?;
        let validated: Vec<(&StagedItem, PathBuf)> = items
            .iter()
            .map(|item| staging::item_path(item, &current).map(|path| (item, path)))
            .collect::<Result<_, _>>()?;
        let mut removed_ids = Vec::new();
        let mut failed = Vec::new();
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
        if !failed.is_empty() {
            if !removed_ids.is_empty() {
                let mut connection = state.db.lock().unwrap();
                let transaction = connection
                    .transaction()
                    .map_err(|error| error.to_string())?;
                db::delete_items_by_ids(&transaction, &removed_ids)?;
                transaction.commit().map_err(|error| error.to_string())?;
                state.mark_staged();
                events::emit_items_changed(&app, pod_id);
            }
            return Err(format!("部分文件无法移入回收站：{}", failed.join("；")));
        }
    }

    // 即使保留文件也要删除记录，避免后续匣因旧 ID 复用继承这些条目。
    {
        let mut connection = state.db.lock().unwrap();
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        db::delete_items_by_pod(&transaction, pod_id as i64)?;
        settings::delete_pod(&transaction, pod_id, &staging::data_dir(&state), VERSION)?;
        transaction.commit().map_err(|error| error.to_string())?;
    }
    state.mark_staged();
    drop(file_operation);
    manager::apply_settings(&app, &manager::current_settings(&app));
    let _ = app.emit(events::PODS_CHANGED, ());
    Ok(())
}

pub fn save_settings(app: AppHandle, patch: serde_json::Value) -> Result<Settings, String> {
    let state = app.state::<AppState>();
    let _operation = state.settings_ops.lock().unwrap();
    let (previous, next) = {
        let connection = state.db.lock().unwrap();
        let previous = staging::load_settings_from(&connection, &state)?;
        let next =
            settings::merge_persist(&connection, patch, &staging::data_dir(&state), VERSION)?;
        (previous, next)
    };
    let hotkeys_changed = next.hotkeys.toggle_bar != previous.hotkeys.toggle_bar
        || next.hotkeys.collect_clipboard != previous.hotkeys.collect_clipboard
        || next.hotkeys.open_panel != previous.hotkeys.open_panel;
    if hotkeys_changed {
        if let Err(error) = crate::hotkeys::register(&app, &next) {
            let restore_hotkeys = crate::hotkeys::register(&app, &previous).err();
            let restore_settings = {
                let connection = state.db.lock().unwrap();
                settings::persist(&connection, &previous).err()
            };
            if restore_settings.is_none() {
                let _ = app.emit(events::SETTINGS_CHANGED, previous);
            }
            let mut errors = vec![error];
            if let Some(error) = restore_hotkeys {
                errors.push(format!("恢复旧快捷键也失败：{error}"));
            }
            if let Some(error) = restore_settings {
                errors.push(format!("恢复旧设置也失败：{error}"));
            }
            return Err(errors.join("；"));
        }
    }
    if next.autostart != previous.autostart {
        if let Err(error) = manager::sync_autostart(&app, next.autostart) {
            let mut errors = vec![error];
            if let Err(error) = manager::sync_autostart(&app, previous.autostart) {
                errors.push(format!("恢复旧自启动状态也失败：{error}"));
            }
            if hotkeys_changed {
                if let Err(error) = crate::hotkeys::register(&app, &previous) {
                    errors.push(format!("恢复旧快捷键也失败：{error}"));
                }
            }
            let restore = {
                let connection = state.db.lock().unwrap();
                settings::persist(&connection, &previous)
            };
            match restore {
                Ok(()) => {
                    let _ = app.emit(events::SETTINGS_CHANGED, previous.clone());
                }
                Err(error) => errors.push(format!("恢复旧设置也失败：{error}")),
            }
            return Err(errors.join("；"));
        }
    }
    manager::apply_settings(&app, &next);
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_accepts_legacy_numeric_strings_and_rejects_bad_fields() {
        let pod = from_config(&serde_json::json!({
            "name": "我的匣",
            "edge": "left",
            "stagingFolder": "D:\\暂存",
            "opacity": "0.85",
            "panelWidth": "380"
        }))
        .unwrap();
        assert_eq!(pod.opacity, 0.85);
        assert_eq!(pod.panel_width, 380);

        let mut pod = Pod::default();
        assert!(apply_patch(&mut pod, &serde_json::json!({ "enabled": "yes" })).is_err());
        assert!(apply_patch(&mut pod, &serde_json::json!({ "panelWidth": -1 })).is_err());
        assert!(apply_patch(&mut pod, &serde_json::json!({ "typo": true })).is_err());
    }

    #[test]
    fn equivalent_folder_spelling_is_not_a_change() {
        let temporary = tempfile::tempdir().unwrap();
        let plain = temporary.path().to_string_lossy();
        let dotted = temporary.path().join(".").to_string_lossy().into_owned();
        assert!(!staging_folder_changed(&plain, &dotted).unwrap());
    }
}
