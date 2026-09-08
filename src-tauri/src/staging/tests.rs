use std::fs;

use crate::db::{self, StagedItem};
use crate::file_ops::{self};
use crate::state::AppState;

use super::recovery::recover_pending_moves_state;
use super::text::{sanitize_text_name, text_file_base};
use super::*;

#[test]
fn text_names_keep_the_readable_head_and_one_txt_suffix() {
    assert_eq!(sanitize_text_name("héllo world"), "héllo world");
    assert_eq!(sanitize_text_name("a<b>c:d"), "a b c d");
    assert_eq!(sanitize_text_name("   "), "文字");
    assert_eq!(sanitize_text_name(&"字".repeat(80)).chars().count(), 48);
    assert_eq!(text_file_base(Some("实验记录"), "正文"), "实验记录");
    assert_eq!(text_file_base(Some("实验记录.txt"), "正文"), "实验记录");
    assert_eq!(text_file_base(Some("  "), "第一行\n第二行"), "第一行");
}

#[test]
fn reserved_device_names_are_prefixed_for_win32() {
    assert_eq!(sanitize_text_name("CON"), "_CON");
    assert_eq!(text_file_base(Some("nul"), "正文"), "_nul");
    assert_eq!(text_file_base(Some("com1"), "正文"), "_com1");
    // 只有“点号前的主干”完全等于保留名才需要规避；
    // 常规名称、只是包含保留词的名字不受影响
    assert_eq!(sanitize_text_name("console"), "console");
    assert_eq!(sanitize_text_name("COM1 调试"), "COM1 调试");
    assert_eq!(sanitize_text_name("我的 CON 记录"), "我的 CON 记录");
}

#[test]
fn interrupted_cross_volume_moves_recover_from_ledger() {
    let temporary = tempfile::tempdir().unwrap();
    let pod_root = temporary.path().join("stage");
    fs::create_dir_all(&pod_root).unwrap();
    let source_dir = temporary.path().join("src");
    fs::create_dir_all(&source_dir).unwrap();
    let original = source_dir.join("报告.docx");
    fs::write(&original, b"payload").unwrap();

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    db::migrate(&conn).unwrap();
    let quarantine = source_dir.join(".floepod-move-source-1-0000000000000001");
    file_ops::rename_new(&original, &quarantine).unwrap();
    db::insert_pending_move(
        &conn,
        &db::PendingMove {
            quarantine_path: quarantine.to_string_lossy().to_string(),
            original_path: original.to_string_lossy().to_string(),
            target_path: pod_root.join("报告.docx").to_string_lossy().to_string(),
        },
    )
    .unwrap();

    // 目标未入库 -> 未提交 -> 恢复原文件
    let state = AppState::new(conn, temporary.path().join("data"));
    recover_pending_moves_state(&state);
    assert!(original.is_file());
    assert!(!quarantine.exists());
    assert!(load_settings(&state).is_ok());
}

#[test]
fn committed_moves_drop_the_quarantine_copy_on_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let source_dir = temporary.path().join("src");
    fs::create_dir_all(&source_dir).unwrap();
    let quarantine = source_dir.join(".floepod-move-source-1-0000000000000002");
    fs::write(&quarantine, b"stale copy").unwrap();

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    db::migrate(&conn).unwrap();
    let target = quarantine
        .with_extension("staged")
        .to_string_lossy()
        .to_string();
    fs::write(&target, b"stale copy").unwrap();
    db::insert_item(
        &conn,
        &StagedItem {
            id: 0,
            pod_id: 1,
            kind: "file".into(),
            staging_path: target.clone(),
            original_path: None,
            name: "a.staged".into(),
            ext: Some("staged".into()),
            size: 0,
            created_at: db::now_ms(),
        },
    )
    .unwrap();
    db::insert_pending_move(
        &conn,
        &db::PendingMove {
            quarantine_path: quarantine.to_string_lossy().to_string(),
            original_path: source_dir.join("a.bin").to_string_lossy().to_string(),
            target_path: target,
        },
    )
    .unwrap();

    // 目标已在库 -> 已提交 -> 删除残留副本
    let state = AppState::new(conn, temporary.path().join("data"));
    recover_pending_moves_state(&state);
    assert!(!quarantine.exists());
    let connection = state.db.lock().unwrap();
    assert!(db::list_pending_moves(&connection).unwrap().is_empty());
}

#[test]
fn committed_move_with_missing_target_restores_its_only_copy() {
    let temporary = tempfile::tempdir().unwrap();
    let source_dir = temporary.path().join("src");
    fs::create_dir_all(&source_dir).unwrap();
    let quarantine = source_dir.join(".floepod-move-source-1-0000000000000002");
    fs::write(&quarantine, b"stale copy").unwrap();

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    db::migrate(&conn).unwrap();
    let target = quarantine
        .with_extension("staged")
        .to_string_lossy()
        .to_string();
    // The database row survived, but the destination did not.
    db::insert_item(
        &conn,
        &StagedItem {
            id: 0,
            pod_id: 1,
            kind: "file".into(),
            staging_path: target.clone(),
            original_path: None,
            name: "a.staged".into(),
            ext: Some("staged".into()),
            size: 0,
            created_at: db::now_ms(),
        },
    )
    .unwrap();
    db::insert_pending_move(
        &conn,
        &db::PendingMove {
            quarantine_path: quarantine.to_string_lossy().to_string(),
            original_path: source_dir.join("a.bin").to_string_lossy().to_string(),
            target_path: target,
        },
    )
    .unwrap();

    // 目标已在库 -> 已提交 -> 删除残留副本
    let state = AppState::new(conn, temporary.path().join("data"));
    recover_pending_moves_state(&state);
    assert!(!quarantine.exists());
    assert_eq!(fs::read(source_dir.join("a.bin")).unwrap(), b"stale copy");
    let connection = state.db.lock().unwrap();
    assert!(db::list_pending_moves(&connection).unwrap().is_empty());
}

#[test]
fn committed_move_with_changed_target_preserves_both_files_and_ledger() {
    let temporary = tempfile::tempdir().unwrap();
    let source_dir = temporary.path().join("src");
    fs::create_dir_all(&source_dir).unwrap();
    let quarantine = source_dir.join(".floepod-move-source-1-0000000000000002");
    fs::write(&quarantine, b"stale copy").unwrap();

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    db::migrate(&conn).unwrap();
    let target = quarantine
        .with_extension("staged")
        .to_string_lossy()
        .to_string();
    fs::write(&target, b"changed target").unwrap();
    db::insert_item(
        &conn,
        &StagedItem {
            id: 0,
            pod_id: 1,
            kind: "file".into(),
            staging_path: target.clone(),
            original_path: None,
            name: "a.staged".into(),
            ext: Some("staged".into()),
            size: 0,
            created_at: db::now_ms(),
        },
    )
    .unwrap();
    db::insert_pending_move(
        &conn,
        &db::PendingMove {
            quarantine_path: quarantine.to_string_lossy().to_string(),
            original_path: source_dir.join("a.bin").to_string_lossy().to_string(),
            target_path: target,
        },
    )
    .unwrap();

    // 目标已在库 -> 已提交 -> 删除残留副本
    let state = AppState::new(conn, temporary.path().join("data"));
    recover_pending_moves_state(&state);
    assert_eq!(fs::read(&quarantine).unwrap(), b"stale copy");
    let connection = state.db.lock().unwrap();
    assert_eq!(db::list_pending_moves(&connection).unwrap().len(), 1);
}
