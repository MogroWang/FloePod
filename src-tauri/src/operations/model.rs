//! 持久化操作时间线、补偿动作与 24 小时基础撤销。
//!
//! 文件操作先完成自身的原子提交，再把可逆步骤写入 operations / operation_items /
//! compensations。历史写入失败不能反向破坏已经成功的文件操作，因此调用方应记录
//! 日志并把操作结果照常返回；撤销则始终保守校验文件身份，内容已变化时拒绝删除。

use serde::Serialize;
use serde_json::Value;

use crate::db::{self};

pub const BASIC_UNDO_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone)]
pub struct CompensationDraft {
    pub kind: String,
    pub source_path: Option<String>,
    pub target_path: Option<String>,
    pub expected_signature: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OperationItemDraft {
    pub item_id: Option<i64>,
    pub name: String,
    pub source_path: Option<String>,
    pub target_path: Option<String>,
    pub action: String,
    pub status: String,
    pub error: Option<String>,
    pub snapshot: Option<String>,
    pub compensation: Option<CompensationDraft>,
}

#[derive(Debug, Clone)]
pub struct OperationDraft {
    pub kind: String,
    pub pod_id: Option<i64>,
    pub summary: String,
    pub status: String,
    pub undoable_until: Option<i64>,
    pub metadata: Value,
    pub items: Vec<OperationItemDraft>,
}

impl OperationDraft {
    pub fn completed(
        kind: impl Into<String>,
        pod_id: Option<i64>,
        summary: impl Into<String>,
        metadata: Value,
        items: Vec<OperationItemDraft>,
    ) -> Self {
        Self {
            kind: kind.into(),
            pod_id,
            summary: summary.into(),
            status: "completed".into(),
            undoable_until: Some(db::now_ms().saturating_add(BASIC_UNDO_MS)),
            metadata,
            items,
        }
    }
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OperationItemEntry {
    pub id: i64,
    pub item_id: Option<i64>,
    pub name: String,
    pub source_path: Option<String>,
    pub target_path: Option<String>,
    pub action: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OperationEntry {
    pub id: i64,
    pub kind: String,
    pub pod_id: Option<i64>,
    pub summary: String,
    pub status: String,
    pub created_at: i64,
    pub undoable_until: Option<i64>,
    pub undone_at: Option<i64>,
    pub undoable: bool,
    pub retryable: bool,
    pub items: Vec<OperationItemEntry>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UndoResult {
    pub operation_id: i64,
    pub restored: usize,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetryResult {
    pub operation_id: i64,
    pub kind: String,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OperationPreview {
    pub title: String,
    pub details: Vec<String>,
    pub warnings: Vec<String>,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone)]
pub(super) struct StoredCompensation {
    pub(super) id: i64,
    pub(super) item_id: Option<i64>,
    pub(super) pod_id: Option<i64>,
    pub(super) name: String,
    pub(super) kind: String,
    pub(super) source_path: Option<String>,
    pub(super) target_path: Option<String>,
    pub(super) expected_signature: Option<String>,
    pub(super) snapshot: Option<String>,
}
