use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::db::{self};
use crate::file_ops::{self};
use crate::state::AppState;

use super::context::*;
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
    file_ops::rename_new(&quarantine, &destination).map_err(|error| {
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
pub(super) fn recover_pending_moves_state(state: &AppState) {
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
                continue;
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
            match db::find_by_path(&connection, &record.target_path) {
                Ok(found) => found.is_some(),
                Err(error) => {
                    crate::logging::write(&format!(
                        "[recovery] 无法判断移动是否提交，保留隔离件与台账: {error}"
                    ));
                    continue;
                }
            }
        };
        let outcome = if committed {
            let target = Path::new(&record.target_path);
            match fs::symlink_metadata(target) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    restore_quarantine(&record)
                        .map(|path| format!("已提交目标丢失，保留副本已恢复至 {}", path.display()))
                }
                _ => verified_cleanup(&quarantine, target),
            }
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

fn verified_cleanup(quarantine: &Path, target: &Path) -> Result<String, String> {
    let source_hash = crate::file_fingerprint::content(quarantine)?;
    let target_hash = crate::file_fingerprint::content(target)?;
    if source_hash != target_hash {
        return Err("已提交目标内容与隔离副本不同，保留副本与台账供恢复".into());
    }
    file_ops::remove_path(quarantine).map(|_| "已校验并清理已提交移动的隔离副本".into())
}

/// 历史版本（1.0.0 及更早）没有台账，遗留在暂存目录里的内部临时文件
/// 无法可靠还原（原名已丢失），只留痕提醒用户手动处理。
fn scan_legacy_stranded(state: &AppState) {
    let Ok(current) = load_settings(state) else {
        return;
    };
    for pod in current.pods.iter().filter(|pod| pod.enabled) {
        let Ok(root) = crate::file_paths::resolve_path(Path::new(&pod.staging_folder)) else {
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
