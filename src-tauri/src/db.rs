use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Row};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn open(dir: &Path) -> Result<Connection, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("无法创建数据目录: {e}"))?;
    let conn = Connection::open(dir.join("data.db")).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    migrate(&conn)?;
    Ok(conn)
}

/// 0.4 迁移：删除「场景」，条目归属改为 pod_id（旧数据全部归入默认匣 1）。
pub fn migrate(conn: &Connection) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let has_scenes: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='scenes'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);

    let item_cols: Vec<String> = tx
        .prepare("PRAGMA table_info(items)")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // 旧版（0.2/0.3）：scene_id -> pod_id，全部归入默认匣 1，并删除场景表
    if has_scenes {
        if item_cols.iter().any(|c| c == "scene_id") {
            tx.execute_batch(
                "DROP INDEX IF EXISTS idx_items_scene;
                 ALTER TABLE items RENAME COLUMN scene_id TO pod_id;
                 UPDATE items SET pod_id = 1;
                 DROP TABLE IF EXISTS scenes;",
            )
            .map_err(|e| e.to_string())?;
        } else {
            tx.execute_batch("DROP TABLE IF EXISTS scenes;")
                .map_err(|e| e.to_string())?;
        }
    }

    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS items (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          pod_id INTEGER NOT NULL DEFAULT 1,
          kind TEXT NOT NULL,
          staging_path TEXT NOT NULL UNIQUE,
          original_path TEXT,
          name TEXT NOT NULL,
          ext TEXT,
          size INTEGER NOT NULL DEFAULT 0,
          created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_items_pod ON items(pod_id);
        CREATE TABLE IF NOT EXISTS settings (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/* ---------- 类型 ---------- */

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedItem {
    pub id: i64,
    pub pod_id: i64,
    pub kind: String,
    pub staging_path: String,
    pub original_path: Option<String>,
    pub name: String,
    pub ext: Option<String>,
    pub size: i64,
    pub created_at: i64,
}

fn item_from_row(row: &Row) -> rusqlite::Result<StagedItem> {
    Ok(StagedItem {
        id: row.get(0)?,
        pod_id: row.get(1)?,
        kind: row.get(2)?,
        staging_path: row.get(3)?,
        original_path: row.get(4)?,
        name: row.get(5)?,
        ext: row.get(6)?,
        size: row.get(7)?,
        created_at: row.get(8)?,
    })
}

const ITEM_COLS: &str =
    "id, pod_id, kind, staging_path, original_path, name, ext, size, created_at";

/* ---------- items ---------- */

pub fn insert_item(conn: &Connection, it: &StagedItem) -> Result<StagedItem, String> {
    let inserted = conn.execute(
        "INSERT INTO items (pod_id, kind, staging_path, original_path, name, ext, size, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(staging_path) DO NOTHING",
        params![
            it.pod_id,
            it.kind,
            it.staging_path,
            it.original_path,
            it.name,
            it.ext,
            it.size,
            it.created_at
        ],
    )
    .map_err(|e| e.to_string())?;
    if inserted == 0 {
        let owner = find_by_path(conn, &it.staging_path)?
            .map(|saved| saved.pod_id)
            .unwrap_or_default();
        return Err(if owner == it.pod_id {
            format!("暂存路径已存在于索引: {}", it.staging_path)
        } else {
            format!("暂存路径已属于另一个匣: {}", it.staging_path)
        });
    }
    find_by_path(conn, &it.staging_path)?.ok_or_else(|| "插入后未找到记录".to_string())
}

/// 更新 Watcher 从磁盘重新观测到的字段，保留 original_path 与 created_at。
pub fn update_item_observed(
    conn: &Connection,
    id: i64,
    kind: &str,
    staging_path: &str,
    name: &str,
    ext: Option<&str>,
    size: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE items
         SET kind = ?2, staging_path = ?3, name = ?4, ext = ?5, size = ?6
         WHERE id = ?1",
        params![id, kind, staging_path, name, ext, size],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn find_by_path(conn: &Connection, path: &str) -> Result<Option<StagedItem>, String> {
    conn.query_row(
        &format!("SELECT {ITEM_COLS} FROM items WHERE staging_path = ?1"),
        params![path],
        item_from_row,
    )
    .optional()
    .map_err(|e| e.to_string())
}

#[allow(dead_code)]
pub fn list_items(conn: &Connection) -> Result<Vec<StagedItem>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ITEM_COLS} FROM items ORDER BY created_at DESC, id DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], item_from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn items_of_pod(conn: &Connection, pod_id: i64) -> Result<Vec<StagedItem>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ITEM_COLS} FROM items WHERE pod_id = ?1 ORDER BY created_at DESC, id DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![pod_id], item_from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn items_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<StagedItem>, String> {
    let mut out = Vec::new();
    for id in ids {
        let found: Option<StagedItem> = conn
            .query_row(
                &format!("SELECT {ITEM_COLS} FROM items WHERE id = ?1"),
                params![id],
                item_from_row,
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(i) = found {
            out.push(i);
        }
    }
    Ok(out)
}

pub fn delete_items_by_ids(conn: &Connection, ids: &[i64]) -> Result<(), String> {
    for chunk in ids.chunks(500) {
        if chunk.is_empty() {
            continue;
        }
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        conn.execute(
            &format!("DELETE FROM items WHERE id IN ({placeholders})"),
            rusqlite::params_from_iter(chunk.iter()),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn delete_items_by_pod(conn: &Connection, pod_id: i64) -> Result<Vec<StagedItem>, String> {
    let items = items_of_pod(conn, pod_id)?;
    conn.execute("DELETE FROM items WHERE pod_id = ?1", params![pod_id])
        .map_err(|e| e.to_string())?;
    Ok(items)
}

/* ---------- settings kv ---------- */

pub fn kv_get(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn kv_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/* ---------- tests ---------- */

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&c).unwrap();
        c
    }

    #[test]
    fn fresh_db_has_no_scenes() {
        let c = conn();
        let has_scenes: bool = c
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='scenes'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has_scenes);
    }

    #[test]
    fn insert_and_fetch_item() {
        let c = conn();
        let it = StagedItem {
            id: 0,
            pod_id: 1,
            kind: "file".into(),
            staging_path: "C:\\staging\\a.pdf".into(),
            original_path: Some("C:\\orig\\a.pdf".into()),
            name: "a.pdf".into(),
            ext: Some("pdf".into()),
            size: 1024,
            created_at: now_ms(),
        };
        let saved = insert_item(&c, &it).unwrap();
        assert!(saved.id > 0);
        assert_eq!(items_of_pod(&c, 1).unwrap().len(), 1);
        assert_eq!(
            find_by_path(&c, "C:\\staging\\a.pdf")
                .unwrap()
                .unwrap()
                .name,
            "a.pdf"
        );
    }

    #[test]
    fn duplicate_staging_path_is_an_error() {
        let c = conn();
        let item = StagedItem {
            id: 0,
            pod_id: 1,
            kind: "file".into(),
            staging_path: "C:\\staging\\same.txt".into(),
            original_path: None,
            name: "same.txt".into(),
            ext: Some("txt".into()),
            size: 1,
            created_at: now_ms(),
        };
        insert_item(&c, &item).unwrap();
        assert!(insert_item(&c, &item).is_err());

        let mut other_pod = item;
        other_pod.pod_id = 2;
        assert!(insert_item(&c, &other_pod).is_err());
        assert_eq!(items_of_pod(&c, 1).unwrap().len(), 1);
        assert!(items_of_pod(&c, 2).unwrap().is_empty());
    }

    #[test]
    fn legacy_scene_db_migrates_to_pod() {
        // 模拟 0.3 旧库
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            r#"
            CREATE TABLE scenes (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, sort INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
            CREATE TABLE items (id INTEGER PRIMARY KEY AUTOINCREMENT, scene_id INTEGER NOT NULL, kind TEXT NOT NULL, staging_path TEXT NOT NULL UNIQUE, original_path TEXT, name TEXT NOT NULL, ext TEXT, size INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
            CREATE INDEX idx_items_scene ON items(scene_id);
            INSERT INTO scenes (id, name, sort, created_at) VALUES (1, '默认', 0, 1), (2, '工作', 1, 2);
            INSERT INTO items (scene_id, kind, staging_path, original_path, name, ext, size, created_at)
              VALUES (1,'file','C:\\s\\a.txt',NULL,'a.txt','txt',1,1), (2,'file','C:\\s\\b.pdf',NULL,'b.pdf','pdf',2,2);
            "#,
        )
        .unwrap();
        migrate(&c).unwrap();
        let items = list_items(&c).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.pod_id == 1));
        let has_scenes: bool = c
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='scenes'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has_scenes);
    }

    #[test]
    fn legacy_migration_preserves_settings_and_is_idempotent() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            r#"
            CREATE TABLE scenes (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE items (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              scene_id INTEGER NOT NULL,
              kind TEXT NOT NULL,
              staging_path TEXT NOT NULL UNIQUE,
              original_path TEXT,
              name TEXT NOT NULL,
              ext TEXT,
              size INTEGER NOT NULL DEFAULT 0,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO settings (key, value) VALUES
              ('app', '{"theme":"dark","firstRunDone":true,"stagingFolder":"D:\\stage"}'),
              ('future_key', 'preserve-me');
            INSERT INTO items
              (scene_id, kind, staging_path, original_path, name, ext, size, created_at)
              VALUES (9, 'text', 'D:\\stage\\note.txt', NULL, 'note.txt', 'txt', 4, 42);
            "#,
        )
        .unwrap();

        migrate(&c).unwrap();
        migrate(&c).unwrap();

        assert_eq!(
            kv_get(&c, "future_key").unwrap().as_deref(),
            Some("preserve-me")
        );
        let settings: serde_json::Value =
            serde_json::from_str(&kv_get(&c, "app").unwrap().unwrap()).unwrap();
        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["stagingFolder"], r"D:\stage");
        let items = list_items(&c).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].pod_id, 1);
        assert_eq!(items[0].kind, "text");
        assert_eq!(items[0].created_at, 42);
    }

    #[test]
    fn deleting_more_than_one_sqlite_parameter_batch_is_complete() {
        let c = conn();
        for index in 0..1_025 {
            insert_item(
                &c,
                &StagedItem {
                    id: 0,
                    pod_id: 1,
                    kind: "file".into(),
                    staging_path: format!(r"C:\stage\{index}.txt"),
                    original_path: None,
                    name: format!("{index}.txt"),
                    ext: Some("txt".into()),
                    size: index,
                    created_at: index,
                },
            )
            .unwrap();
        }
        let ids: Vec<_> = list_items(&c)
            .unwrap()
            .into_iter()
            .map(|item| item.id)
            .collect();
        delete_items_by_ids(&c, &ids).unwrap();
        assert!(list_items(&c).unwrap().is_empty());
    }
}
