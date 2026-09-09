//! 持久化操作时间线、补偿动作与 24 小时基础撤销。
//!
//! 文件操作先完成自身的原子提交，再把可逆步骤写入 operations / operation_items /
//! compensations。历史写入失败不能反向破坏已经成功的文件操作，因此调用方应记录
//! 日志并把操作结果照常返回；撤销则始终保守校验文件身份，内容已变化时拒绝删除。

use std::fs;

use rusqlite::Connection;

use crate::db::{self};

use super::store::list_from;
use super::undo::move_for_restore;
use super::*;

fn connection() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    db::migrate(&conn).unwrap();
    conn
}

#[test]
fn operation_round_trip_exposes_undo_and_retry_flags() {
    let conn = connection();
    let id = record(
        &conn,
        OperationDraft::completed(
            "stage",
            Some(1),
            "复制 1 项到测试匣",
            serde_json::json!({"retry":{"podId":1,"paths":["C:\\a.txt"],"action":"copy"}}),
            vec![OperationItemDraft {
                item_id: Some(7),
                name: "a.txt".into(),
                source_path: Some("C:\\a.txt".into()),
                target_path: Some("D:\\pod\\a.txt".into()),
                action: "copy".into(),
                status: "completed".into(),
                error: None,
                snapshot: None,
                compensation: Some(CompensationDraft {
                    kind: "delete_staged_copy".into(),
                    source_path: None,
                    target_path: Some("D:\\pod\\a.txt".into()),
                    expected_signature: Some("abc".into()),
                }),
            }],
        ),
    )
    .unwrap();
    assert!(id > 0);
    let rows = list_from(&conn, 0, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].undoable);
    assert!(rows[0].retryable);
    assert_eq!(rows[0].items[0].name, "a.txt");
}

#[test]
fn signature_changes_when_file_content_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("a.txt");
    fs::write(&path, b"one").unwrap();
    let before = signature(&path).unwrap();
    fs::write(&path, b"a different value").unwrap();
    let after = signature(&path).unwrap();
    assert_ne!(before, after);
}

#[test]
fn restore_uses_a_non_conflicting_name() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("quarantine.txt");
    let target = temporary.path().join("restored.txt");
    fs::write(&source, b"restored").unwrap();
    fs::write(&target, b"existing").unwrap();
    let actual = move_for_restore(&source, &target).unwrap();
    assert_ne!(actual, target);
    assert_eq!(fs::read(actual).unwrap(), b"restored");
    assert_eq!(fs::read(target).unwrap(), b"existing");
}

#[test]
fn expired_operations_are_not_undoable() {
    let conn = connection();
    conn.execute(
        "INSERT INTO operations
             (kind, summary, status, created_at, undoable_until, metadata)
             VALUES ('remove', '旧操作', 'completed', 1, 2, '{}')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO operation_items
             (operation_id, name, action, status) VALUES (1, 'a', 'remove', 'completed')",
        [],
    )
    .unwrap();
    let rows = list_from(&conn, 0, 10).unwrap();
    assert!(!rows[0].undoable);
}

#[test]
fn retention_uses_supplied_time_and_retries_failed_items_without_losing_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("undo");
    fs::create_dir(&root).unwrap();
    let path = root.join("saved.txt");
    fs::write(&path, b"original").unwrap();
    let expected = signature(&path).unwrap();
    let conn = connection();
    conn.execute("INSERT INTO operations (kind, summary, status, created_at, undoable_until, metadata) VALUES ('remove', 'expiry', 'completed', 1, 100, '{}')", []).unwrap();
    conn.execute("INSERT INTO operation_items (operation_id, name, action, status) VALUES (1, 'saved.txt', 'remove', 'completed')", []).unwrap();
    conn.execute("INSERT INTO compensations (operation_item_id, kind, source_path, expected_signature) VALUES (1, 'restore_removed_file', ?1, ?2)", rusqlite::params![path.to_string_lossy(), expected]).unwrap();
    let state = crate::state::AppState::new(conn, temporary.path().to_path_buf());
    let stop = crate::lifecycle::StopToken::default();
    let status = || {
        state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT status FROM compensations WHERE id = 1", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap()
    };
    super::retention::purge_expired(&state, &stop, 100).unwrap();
    assert_eq!(status(), "pending");
    fs::write(&path, b"changed by user").unwrap();
    assert!(super::retention::purge_expired(&state, &stop, 101).is_err());
    assert_eq!(status(), "pending");
    assert_eq!(fs::read(&path).unwrap(), b"changed by user");
    // The user independently removed the changed file. The next tick can finish
    // the ledger update without deleting any replacement or needing a restart.
    fs::remove_file(&path).unwrap();
    super::retention::purge_expired(&state, &stop, 102).unwrap();
    assert_eq!(status(), "expired");
    super::retention::purge_expired(&state, &stop, 103).unwrap();
}

#[test]
fn missing_identity_refuses_undo_but_existing_legacy_signatures_still_work() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("saved.txt");
    fs::write(&path, b"original").unwrap();
    assert!(super::identity::verify_signature(&path, None).is_err());
    let signature = signature(&path).unwrap();
    let legacy = signature.rsplit(':').next().unwrap();
    super::identity::verify_signature(&path, Some(legacy)).unwrap();
    super::identity::verify_signature(&path, Some(&signature)).unwrap();
}
