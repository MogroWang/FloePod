//! 持久化操作时间线、补偿动作与 24 小时基础撤销。
//!
//! 文件操作先完成自身的原子提交，再把可逆步骤写入 operations / operation_items /
//! compensations。历史写入失败不能反向破坏已经成功的文件操作，因此调用方应记录
//! 日志并把操作结果照常返回；撤销则始终保守校验文件身份，内容已变化时拒绝删除。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::params;

use crate::db::{self, StagedItem};
use crate::file_ops;
use crate::state::AppState;

use super::undo::move_for_restore;
pub fn undo_root(state: &AppState) -> PathBuf {
    state.data_dir.join("undo")
}

/// Caller holds file_ops for the entire snapshot/cleanup/update sequence.
pub fn purge_expired(
    state: &AppState,
    stop: &crate::lifecycle::StopToken,
    now: i64,
) -> Result<(), String> {
    let expired = {
        let conn = state.db.lock().unwrap();
        let mut statement = conn
            .prepare(
                "SELECT c.id, c.source_path, c.expected_signature FROM compensations c
             JOIN operation_items oi ON oi.id = c.operation_item_id
             JOIN operations o ON o.id = oi.operation_id
             WHERE c.kind = 'restore_removed_file' AND c.status = 'pending'
               AND o.undoable_until IS NOT NULL AND o.undoable_until < ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![now], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
    };
    let root = crate::file_paths::resolve_path(&undo_root(state))?;
    let mut failures = Vec::new();
    for (id, path, expected) in expired {
        if stop.is_stopped() {
            break;
        }
        let outcome = (|| {
            let path = PathBuf::from(path.ok_or("过期撤销记录缺少文件路径")?);
            let resolved = crate::file_paths::resolve_path(&path)?;
            if !crate::file_paths::path_is_within(&resolved, &root)
                || crate::file_paths::paths_equal(&resolved, &root)
            {
                return Err("过期撤销路径不属于应用撤销区".into());
            }
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
                Ok(_) => {
                    super::identity::verify_signature(&path, expected.as_deref())?;
                    trash::delete(&path).map_err(|error| error.to_string())?;
                }
            }
            state
                .db
                .lock()
                .unwrap()
                .execute(
                    "UPDATE compensations SET status = 'expired' WHERE id = ?1",
                    params![id],
                )
                .map_err(|error| error.to_string())?;
            Ok(())
        })();
        if let Err(error) = outcome {
            failures.push(format!("撤销项 {id}: {error}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("；"))
    }
}

pub fn remove_to_undo_store(
    state: &AppState,
    item: &StagedItem,
    path: &Path,
) -> Result<PathBuf, String> {
    let batch = format!("{}-{}", db::now_ms(), std::process::id());
    let root = undo_root(state).join(batch);
    fs::create_dir_all(&root)
        .map_err(|error| format!("无法创建可撤销区 {}: {error}", root.display()))?;
    let target = file_ops::unique_target(&root, &item.name, &mut HashSet::new())?;
    move_for_restore(path, &target)
}
