//! 持久化操作时间线、补偿动作与 24 小时基础撤销。
//!
//! 文件操作先完成自身的原子提交，再把可逆步骤写入 operations / operation_items /
//! compensations。历史写入失败不能反向破坏已经成功的文件操作，因此调用方应记录
//! 日志并把操作结果照常返回；撤销则始终保守校验文件身份，内容已变化时拒绝删除。

use std::fs;
use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::db::{self};
use crate::security;
use crate::staging;
use crate::state::AppState;

use super::model::*;
pub fn preview_remove(
    app: &AppHandle,
    ids: &[i64],
    delete_files: bool,
) -> Result<OperationPreview, String> {
    let state = app.state::<AppState>();
    let (settings, items) = {
        let connection = state.db.lock().unwrap();
        (
            staging::load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, ids)?,
        )
    };
    staging::validate_item_pods(&settings, &state, &items)?;
    security::require_items_unlocked(app, &items)?;
    let details = items
        .iter()
        .map(|item| format!("{} — {}", item.name, item.staging_path))
        .collect::<Vec<_>>();
    let warnings = if delete_files {
        vec!["文件会先进入 FloePod 的 24 小时可撤销区；到期清理时再移入系统回收站。".into()]
    } else {
        vec!["只移除索引，原文件仍留在暂存文件夹中。".into()]
    };
    Ok(OperationPreview {
        title: format!("将从暂存中移出 {} 项", items.len()),
        details,
        warnings,
        requires_confirmation: delete_files || items.len() > 1,
    })
}

pub fn preview_export(
    app: &AppHandle,
    ids: &[i64],
    destination: &str,
    mode: &str,
) -> Result<OperationPreview, String> {
    let state = app.state::<AppState>();
    let (settings, items) = {
        let connection = state.db.lock().unwrap();
        (
            staging::load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, ids)?,
        )
    };
    staging::validate_item_pods(&settings, &state, &items)?;
    security::require_items_unlocked(app, &items)?;
    let destination = PathBuf::from(destination);
    if !destination.is_absolute() {
        return Err("目标文件夹必须是绝对路径".into());
    }
    let mut warnings = Vec::new();
    let details = items
        .iter()
        .map(|item| {
            let target = destination.join(&item.name);
            if fs::symlink_metadata(&target).is_ok() {
                warnings.push(format!("目标已存在同名项：{}", item.name));
            }
            format!("{} → {}", item.staging_path, target.display())
        })
        .collect();
    if mode == "move" {
        warnings.push("移动完成后会从当前匣移除；24 小时内可以从操作中心恢复。".into());
    }
    Ok(OperationPreview {
        title: format!(
            "将{} {} 项",
            if mode == "move" { "移动" } else { "复制" },
            items.len()
        ),
        details,
        warnings,
        requires_confirmation: mode == "move" || items.len() > 1,
    })
}
