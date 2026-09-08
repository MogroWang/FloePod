//! Retention policy execution: serialized filesystem changes followed by index/history updates.
use crate::lifecycle::StopToken;
use crate::operations::{self, OperationDraft, OperationItemDraft};
use crate::state::AppState;
use crate::{db, events, staging};
use std::fs;
use tauri::{AppHandle, Manager};

fn effective_days(configured: u32, mandatory: u32) -> u32 {
    match (configured, mandatory) {
        (0, mandatory) => mandatory,
        (configured, 0) => configured,
        (configured, mandatory) => configured.min(mandatory),
    }
}

pub fn start(app: &AppHandle) -> Result<(), String> {
    // Preserve startup recovery -> retention -> watcher ordering.
    let handle = app.clone();
    app.state::<AppState>().retention_task.start_initialized(
        "retention",
        std::time::Duration::from_secs(5 * 60),
        |stop| purge(app, stop, db::now_ms()),
        move |stop| purge(&handle, stop, db::now_ms()),
    )?;
    Ok(())
}

pub fn purge(app: &AppHandle, stop: &StopToken, now: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _file_operation = state.file_ops.lock().unwrap();
    let mut failures = Vec::new();
    if let Err(error) = operations::purge_expired(&state, stop, now) {
        failures.push(error);
    }
    let settings = staging::load_settings(&state).map_err(|error| {
        failures.push(error);
        failures.join("；")
    })?;
    let mandatory_retention = {
        let status = crate::policy::load()?;
        if status.managed {
            status.policy.mandatory_retention_days
        } else {
            0
        }
    };
    let mut changed = Vec::new();
    for pod in settings.pods.iter().filter(|pod| {
        pod.enabled
            && pod.security.enabled
            && (pod.security.retention_days > 0 || mandatory_retention > 0)
    }) {
        if stop.is_stopped() {
            break;
        }
        let retention_days = effective_days(pod.security.retention_days, mandatory_retention);
        let cutoff = now.saturating_sub(retention_days as i64 * 86_400_000);
        let items = match db::items_of_pod(&state.db.lock().unwrap(), pod.id as i64) {
            Ok(items) => items,
            Err(error) => {
                failures.push(error);
                continue;
            }
        };
        let mut deleted = Vec::new();
        for item in items.into_iter().filter(|item| item.created_at <= cutoff) {
            if stop.is_stopped() {
                break;
            }
            let path = match staging::item_path(&item, &settings) {
                Ok(path) => path,
                Err(error) => {
                    failures.push(error);
                    continue;
                }
            };
            let outcome = match fs::symlink_metadata(&path) {
                Ok(metadata) if crate::file_ops::is_reparse_or_symlink(&metadata) => {
                    Err("拒绝清理符号链接或重解析点".into())
                }
                Ok(_) => trash::delete(&path).map_err(|error| error.to_string()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            };
            match outcome {
                Ok(()) => deleted.push(item),
                Err(error) => {
                    failures.push(format!("到期清理 {} 失败: {error}", item.staging_path))
                }
            }
        }
        if !deleted.is_empty() {
            let deleted_ids = deleted.iter().map(|item| item.id).collect::<Vec<_>>();
            let connection = state.db.lock().unwrap();
            match db::delete_items_by_ids(&connection, &deleted_ids) {
                Err(error) => failures.push(error),
                Ok(()) => {
                    let count = deleted.len();
                    let history = OperationDraft {
                        kind: "retention_cleanup".into(),
                        pod_id: Some(pod.id as i64),
                        summary: format!("按保留期清理「{}」中的 {count} 项", pod.name),
                        status: "completed".into(),
                        undoable_until: None,
                        metadata: serde_json::json!({
                            "retentionDays": retention_days,
                            "policyManaged": mandatory_retention > 0,
                        }),
                        items: deleted
                            .into_iter()
                            .map(|item| {
                                let snapshot = operations::snapshot(&item);
                                OperationItemDraft {
                                    item_id: Some(item.id),
                                    name: item.name,
                                    source_path: Some(item.staging_path),
                                    target_path: None,
                                    action: "retention-delete".into(),
                                    status: "completed".into(),
                                    error: None,
                                    snapshot,
                                    compensation: None,
                                }
                            })
                            .collect(),
                    };
                    if let Err(error) = operations::record(&connection, history) {
                        failures.push(format!("记录保留期清理失败: {error}"));
                    }
                    changed.push(pod.id);
                }
            }
        }
    }
    for pod_id in changed {
        events::emit_items_changed(app, pod_id);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("；"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn organization_retention_can_only_shorten_configured_retention() {
        assert_eq!(effective_days(0, 0), 0);
        assert_eq!(effective_days(0, 7), 7);
        assert_eq!(effective_days(30, 0), 30);
        assert_eq!(effective_days(30, 7), 7);
        assert_eq!(effective_days(3, 7), 3);
    }
}
