//! Durable retry ownership: claim once before effects, consume on success, and
//! preserve an interrupted claim for diagnosis instead of duplicating outputs.
use super::model::*;
use crate::state::AppState;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StageRetry {
    pod_id: u64,
    paths: Vec<String>,
    #[serde(default = "copy_mode")]
    action: String,
}
#[derive(Deserialize)]
struct ExportRetry {
    ids: Vec<i64>,
    destination: String,
    #[serde(default = "copy_mode")]
    mode: String,
}
fn copy_mode() -> String {
    "copy".into()
}
enum RetryRequest {
    Stage(StageRetry),
    Export(ExportRetry),
}
fn parse_request(kind: &str, retry: Value) -> Result<RetryRequest, String> {
    match kind {
        "stage" => {
            let request: StageRetry = serde_json::from_value(retry)
                .map_err(|error| format!("暂存重试记录无效: {error}"))?;
            if request.paths.is_empty()
                || request.paths.iter().any(|path| path.is_empty())
                || !["copy", "move", "link"].contains(&request.action.as_str())
            {
                return Err("暂存重试参数无效".into());
            }
            Ok(RetryRequest::Stage(request))
        }
        "export" => {
            let request: ExportRetry = serde_json::from_value(retry)
                .map_err(|error| format!("导出重试记录无效: {error}"))?;
            if request.ids.is_empty()
                || request.ids.iter().any(|id| *id <= 0)
                || request.destination.is_empty()
                || !["copy", "move"].contains(&request.mode.as_str())
            {
                return Err("导出重试参数无效".into());
            }
            Ok(RetryRequest::Export(request))
        }
        _ => Err("该操作类型不支持自动重试".into()),
    }
}
fn claim(conn: &Connection, operation_id: i64) -> Result<(String, Value, RetryRequest), String> {
    let (kind, original): (String, String) = conn
        .query_row(
            "SELECT kind, metadata FROM operations WHERE id = ?1",
            params![operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or("操作记录不存在")?;
    let mut metadata: Value = serde_json::from_str(&original).map_err(|error| error.to_string())?;
    let payload = metadata
        .get("retry")
        .cloned()
        .ok_or("该操作没有可重试项，或上次重试正在执行／中断后尚待核对")?;
    let request = parse_request(&kind, payload.clone())?;
    metadata
        .as_object_mut()
        .ok_or("重试元数据必须是对象")?
        .remove("retry");
    metadata["retryAttempt"] = serde_json::json!({"state":"running", "payload":payload});
    let changed = conn
        .execute(
            "UPDATE operations SET metadata = ?1 WHERE id = ?2 AND metadata = ?3",
            params![metadata.to_string(), operation_id, original],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("该操作正在被另一个请求重试".into());
    }
    Ok((kind, metadata, request))
}
fn finish(
    conn: &Connection,
    operation_id: i64,
    mut metadata: Value,
    outcome: &Result<Value, String>,
) -> Result<(), String> {
    match outcome {
        Ok(result) => {
            metadata["retryAttempt"]["state"] = "completed".into();
            metadata["retryAttempt"]["result"] = result.clone();
        }
        Err(error) => {
            // These use cases either roll back before returning Err, or report
            // partial outcomes through their normal result and new history row.
            metadata["retry"] = metadata["retryAttempt"]["payload"].clone();
            metadata["retryAttempt"]["state"] = "failed".into();
            metadata["retryAttempt"]["error"] = error.clone().into();
        }
    }
    conn.execute(
        "UPDATE operations SET metadata = ?1 WHERE id = ?2",
        params![metadata.to_string(), operation_id],
    )
    .map_err(|error| format!("重试结果未能写入，已保留领取记录以避免重复执行: {error}"))?;
    Ok(())
}
pub fn retry(app: AppHandle, operation_id: i64) -> Result<RetryResult, String> {
    let state = app.state::<AppState>();
    let _permit = state.tasks.enter()?;
    let (kind, metadata, request) = claim(&state.db.lock().unwrap(), operation_id)?;
    let outcome = match request {
        RetryRequest::Stage(request) => {
            crate::staging::stage_paths(app.clone(), request.pod_id, request.paths, request.action)
                .and_then(|result| serde_json::to_value(result).map_err(|error| error.to_string()))
        }
        RetryRequest::Export(request) => crate::export::export_items(
            app.clone(),
            request.ids,
            request.destination,
            request.mode,
            "rename".into(),
        )
        .and_then(|result| serde_json::to_value(result).map_err(|error| error.to_string())),
    };
    finish(&state.db.lock().unwrap(), operation_id, metadata, &outcome)?;
    Ok(RetryResult {
        operation_id,
        kind,
        result: outcome?,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&conn).unwrap();
        conn.execute("INSERT INTO operations (kind, summary, status, created_at, metadata) VALUES ('export', 'failed', 'failed', 1, ?1)", [serde_json::json!({"retry":{"ids":[1],"destination":"C:\\out"}}).to_string()]).unwrap();
        conn
    }
    #[test]
    fn success_and_interrupted_claim_cannot_be_replayed() {
        let conn = fixture();
        let (_, metadata, _) = claim(&conn, 1).unwrap();
        assert!(claim(&conn, 1).is_err());
        finish(&conn, 1, metadata, &Ok(serde_json::json!({"exported":1}))).unwrap();
        assert!(claim(&conn, 1).is_err());
    }
    #[test]
    fn failed_attempt_can_retry_but_malformed_members_are_not_silently_dropped() {
        let conn = fixture();
        let (_, metadata, _) = claim(&conn, 1).unwrap();
        finish(&conn, 1, metadata, &Err("destination unavailable".into())).unwrap();
        assert!(claim(&conn, 1).is_ok());
        assert!(parse_request(
            "export",
            serde_json::json!({"ids":[1,"invalid"],"destination":"C:\\out"})
        )
        .is_err());
        assert!(
            parse_request("stage", serde_json::json!({"podId":1,"paths":["C:\\a",42]})).is_err()
        );
    }
}
