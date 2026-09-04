//! 敏感匣：Windows EFS 静态加密、Windows Hello 解锁、自动锁定与保留期限。
//!
//! 加密完全委托给 Windows EFS 和当前用户证书，不自制算法或保存口令。应用内
//! 解锁状态只驻留内存；进程退出、紧急锁定或超时后需要重新进行系统验证。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::db;
use crate::events;
use crate::operations::{self, OperationDraft, OperationItemDraft};
use crate::settings::Pod;
use crate::staging;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityStatus {
    pub pod_id: u64,
    pub sensitive: bool,
    pub locked: bool,
    pub efs_encrypted: bool,
    pub expires_soon: usize,
}

#[cfg(target_os = "windows")]
fn wide_null(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;

#[cfg(target_os = "windows")]
pub fn is_efs_encrypted(path: &Path) -> bool {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileAttributesW, FILE_ATTRIBUTE_ENCRYPTED, INVALID_FILE_ATTRIBUTES,
    };
    let attributes = unsafe { GetFileAttributesW(wide_null(path).as_ptr()) };
    attributes != INVALID_FILE_ATTRIBUTES && attributes & FILE_ATTRIBUTE_ENCRYPTED != 0
}

#[cfg(not(target_os = "windows"))]
pub fn is_efs_encrypted(_path: &Path) -> bool {
    false
}

#[cfg(target_os = "windows")]
pub fn ensure_efs(path: &Path) -> Result<(), String> {
    use windows_sys::Win32::Storage::FileSystem::EncryptFileW;
    fs::create_dir_all(path)
        .map_err(|error| format!("无法创建敏感匣目录 {}: {error}", path.display()))?;
    if is_efs_encrypted(path) {
        return Ok(());
    }
    let success = unsafe { EncryptFileW(wide_null(path).as_ptr()) };
    if success == 0 || !is_efs_encrypted(path) {
        return Err(format!(
            "Windows 无法为 {} 启用 EFS。请使用支持 EFS 的 NTFS 卷，或将匣放在 BitLocker 加密盘。",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_efs(_path: &Path) -> Result<(), String> {
    Err("敏感匣 EFS 加密仅支持 Windows".into())
}

fn pod(app: &AppHandle, pod_id: u64) -> Result<Pod, String> {
    staging::load_settings(&app.state::<AppState>())?
        .pods
        .into_iter()
        .find(|pod| pod.id == pod_id && pod.enabled)
        .ok_or_else(|| "匣不存在或已停用".to_string())
}

fn deadline(pod: &Pod) -> Instant {
    if pod.security.auto_lock_minutes == 0 {
        Instant::now() + Duration::from_secs(365 * 24 * 60 * 60)
    } else {
        Instant::now() + Duration::from_secs(pod.security.auto_lock_minutes as u64 * 60)
    }
}

pub fn is_locked(app: &AppHandle, pod_id: u64) -> bool {
    let Ok(pod) = pod(app, pod_id) else {
        return true;
    };
    if !pod.security.enabled {
        return false;
    }
    let state = app.state::<AppState>();
    let mut unlocked = state.unlocked_pods.lock().unwrap();
    match unlocked.get(&pod_id).copied() {
        Some(until) if until > Instant::now() => false,
        _ => {
            unlocked.remove(&pod_id);
            true
        }
    }
}

pub fn require_unlocked(app: &AppHandle, pod_id: u64) -> Result<(), String> {
    let pod = pod(app, pod_id)?;
    if !pod.security.enabled {
        return Ok(());
    }
    let state = app.state::<AppState>();
    let mut unlocked = state.unlocked_pods.lock().unwrap();
    match unlocked.get_mut(&pod_id) {
        Some(until) if *until > Instant::now() => {
            *until = deadline(&pod);
            Ok(())
        }
        _ => {
            unlocked.remove(&pod_id);
            Err("SENSITIVE_POD_LOCKED".into())
        }
    }
}

pub fn require_items_unlocked(
    app: &AppHandle,
    items: &[crate::db::StagedItem],
) -> Result<(), String> {
    let mut pod_ids = items
        .iter()
        .map(|item| item.pod_id as u64)
        .collect::<Vec<_>>();
    pod_ids.sort_unstable();
    pod_ids.dedup();
    for pod_id in pod_ids {
        require_unlocked(app, pod_id)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn verify_windows_hello(message: &str) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    };
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

    struct RoGuard;
    impl Drop for RoGuard {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
        .map_err(|error| format!("无法初始化 Windows 凭据验证: {error}"))?;
    let _guard = RoGuard;
    let availability = UserConsentVerifier::CheckAvailabilityAsync()
        .and_then(|operation| operation.get())
        .map_err(|error| format!("无法检查 Windows Hello: {error}"))?;
    if availability != UserConsentVerifierAvailability::Available {
        return Err("Windows Hello 或系统 PIN 尚未配置，无法解锁敏感匣".into());
    }
    let result = UserConsentVerifier::RequestVerificationAsync(&HSTRING::from(message))
        .and_then(|operation| operation.get())
        .map_err(|error| format!("Windows 凭据验证失败: {error}"))?;
    if result == UserConsentVerificationResult::Verified {
        Ok(())
    } else {
        Err("未通过 Windows Hello 验证".into())
    }
}

#[cfg(not(target_os = "windows"))]
fn verify_windows_hello(_message: &str) -> Result<(), String> {
    Err("Windows Hello 仅支持 Windows".into())
}

pub fn unlock(app: &AppHandle, pod_id: u64) -> Result<SecurityStatus, String> {
    let pod = pod(app, pod_id)?;
    if !pod.security.enabled {
        return status(app, pod_id);
    }
    ensure_efs(Path::new(&pod.staging_folder))?;
    if pod.security.require_windows_hello {
        verify_windows_hello(&format!("解锁 FloePod 敏感匣「{}」", pod.name))?;
    }
    app.state::<AppState>()
        .unlocked_pods
        .lock()
        .unwrap()
        .insert(pod_id, deadline(&pod));
    let _ = app.emit_to(
        events::pod_panel_label(pod_id),
        events::POD_LOCK_CHANGED,
        serde_json::json!({ "podId": pod_id, "locked": false }),
    );
    status(app, pod_id)
}

pub fn lock(app: &AppHandle, pod_id: u64) {
    app.state::<AppState>()
        .unlocked_pods
        .lock()
        .unwrap()
        .remove(&pod_id);
    crate::manager::hide_panel(app, pod_id);
    let _ = app.emit_to(
        events::pod_panel_label(pod_id),
        events::POD_LOCK_CHANGED,
        serde_json::json!({ "podId": pod_id, "locked": true }),
    );
}

pub fn lock_all(app: &AppHandle) {
    let ids = app
        .state::<AppState>()
        .unlocked_pods
        .lock()
        .unwrap()
        .keys()
        .copied()
        .collect::<Vec<_>>();
    app.state::<AppState>()
        .unlocked_pods
        .lock()
        .unwrap()
        .clear();
    for id in ids {
        crate::manager::hide_panel(app, id);
        let _ = app.emit_to(
            events::pod_panel_label(id),
            events::POD_LOCK_CHANGED,
            serde_json::json!({ "podId": id, "locked": true }),
        );
    }
}

pub fn status(app: &AppHandle, pod_id: u64) -> Result<SecurityStatus, String> {
    let pod = pod(app, pod_id)?;
    let now = db::now_ms();
    let reminder_days = [pod.rules.expire_days, pod.security.retention_days]
        .into_iter()
        .filter(|days| *days > 0)
        .min()
        .unwrap_or(0);
    let expires_soon = if reminder_days == 0 {
        0
    } else {
        let threshold = now.saturating_sub(reminder_days as i64 * 86_400_000);
        db::items_of_pod(&app.state::<AppState>().db.lock().unwrap(), pod_id as i64)?
            .into_iter()
            .filter(|item| item.created_at <= threshold)
            .count()
    };
    Ok(SecurityStatus {
        pod_id,
        sensitive: pod.security.enabled,
        locked: is_locked(app, pod_id),
        efs_encrypted: pod.security.enabled && is_efs_encrypted(Path::new(&pod.staging_folder)),
        expires_soon,
    })
}

pub fn ensure_configured(settings: &crate::settings::Settings) {
    for pod in settings
        .pods
        .iter()
        .filter(|pod| pod.enabled && pod.security.enabled)
    {
        if let Err(error) = ensure_efs(Path::new(&pod.staging_folder)) {
            crate::logging::write(&format!(
                "[security] 敏感匣「{}」加密检查失败: {error}",
                pod.name
            ));
        }
    }
}

pub fn purge_retention(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(settings) = staging::load_settings(&state) else {
        return;
    };
    let now = db::now_ms();
    let mandatory_retention = crate::policy::load()
        .ok()
        .filter(|status| status.managed)
        .map(|status| status.policy.mandatory_retention_days)
        .unwrap_or(0);
    let mut changed = Vec::new();
    let _file_operation = state.file_ops.lock().unwrap();
    for pod in settings.pods.iter().filter(|pod| {
        pod.enabled
            && pod.security.enabled
            && (pod.security.retention_days > 0 || mandatory_retention > 0)
    }) {
        let retention_days = match (pod.security.retention_days, mandatory_retention) {
            (0, mandatory) => mandatory,
            (configured, 0) => configured,
            (configured, mandatory) => configured.min(mandatory),
        };
        let cutoff = now.saturating_sub(retention_days as i64 * 86_400_000);
        let items = match db::items_of_pod(&state.db.lock().unwrap(), pod.id as i64) {
            Ok(items) => items,
            Err(_) => continue,
        };
        let mut deleted = Vec::new();
        for item in items.into_iter().filter(|item| item.created_at <= cutoff) {
            let path = PathBuf::from(&item.staging_path);
            let outcome = match fs::symlink_metadata(&path) {
                Ok(_) => trash::delete(&path).map_err(|error| error.to_string()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            };
            match outcome {
                Ok(()) => deleted.push(item),
                Err(error) => crate::logging::write(&format!(
                    "[security] 到期清理 {} 失败: {error}",
                    item.staging_path
                )),
            }
        }
        if !deleted.is_empty() {
            let deleted_ids = deleted.iter().map(|item| item.id).collect::<Vec<_>>();
            let connection = state.db.lock().unwrap();
            if db::delete_items_by_ids(&connection, &deleted_ids).is_ok() {
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
                    crate::logging::write(&format!("[security] 记录保留期清理失败: {error}"));
                }
                changed.push(pod.id);
            }
        }
    }
    for pod_id in changed {
        let _ = app.emit_to(
            events::pod_panel_label(pod_id),
            events::ITEMS_CHANGED,
            serde_json::json!({ "podId": pod_id }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_sensitive_settings_do_not_report_efs_for_missing_path() {
        assert!(!is_efs_encrypted(Path::new(
            r"Z:\definitely-missing-floepod"
        )));
    }
}
