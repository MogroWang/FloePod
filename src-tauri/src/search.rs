//! 完全本地的文件名、备注、标签、文档文本与 Windows OCR 索引。

use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use zip::ZipArchive;

use crate::db::{self, StagedItem};
use crate::policy;
use crate::security;
use crate::staging;
use crate::state::AppState;

const MAX_TEXT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_INDEX_CHARS: usize = 200_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub item: StagedItem,
    pub tags: Vec<String>,
    pub note: String,
    pub snippet: String,
    pub matched_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexResult {
    pub indexed: usize,
    pub skipped: usize,
    pub failures: Vec<String>,
    pub ocr_available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    pub tags: Vec<String>,
    pub note: String,
}

fn strip_xml(xml: &str) -> String {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static ENTITIES: OnceLock<Regex> = OnceLock::new();
    let tags = TAGS.get_or_init(|| Regex::new(r"(?s)<[^>]*>").unwrap());
    let entities = ENTITIES.get_or_init(|| Regex::new(r"&(?:#\d+|#x[0-9a-fA-F]+|\w+);").unwrap());
    let value = tags.replace_all(xml, " ");
    entities.replace_all(&value, " ").to_string()
}

fn read_text_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let text = String::from_utf8(bytes)
        .or_else(|error| {
            let bytes = error.into_bytes();
            if bytes.len() % 2 == 0 {
                let words = bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>();
                String::from_utf16(&words).map_err(|_| ())
            } else {
                Err(())
            }
        })
        .unwrap_or_default();
    Ok(text)
}

fn extract_office(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|error| error.to_string())?;
    let mut parts = Vec::new();
    let mut indexed_chars = 0usize;
    for index in 0..archive.len() {
        if indexed_chars >= MAX_INDEX_CHARS {
            break;
        }
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let name = entry.name().replace('\\', "/");
        let include = name == "word/document.xml"
            || name == "xl/sharedStrings.xml"
            || name.starts_with("ppt/slides/slide") && name.ends_with(".xml")
            || name == "content.xml";
        if !include || entry.size() > MAX_TEXT_BYTES {
            continue;
        }
        let mut xml = String::new();
        entry
            .read_to_string(&mut xml)
            .map_err(|error| error.to_string())?;
        let text = strip_xml(&xml)
            .chars()
            .take(MAX_INDEX_CHARS.saturating_sub(indexed_chars))
            .collect::<String>();
        indexed_chars += text.chars().count();
        parts.push(text);
    }
    Ok(parts.join("\n"))
}

fn extract_pdf(path: &Path) -> Result<String, String> {
    let document = lopdf::Document::load(path).map_err(|error| error.to_string())?;
    let pages = document
        .get_pages()
        .keys()
        .copied()
        .take(100)
        .collect::<Vec<_>>();
    document
        .extract_text_with_limit(&pages, 16 * 1024 * 1024)
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn ocr_image(path: &Path) -> Result<String, String> {
    use windows::core::HSTRING;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::{FileAccessMode, StorageFile};
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

    struct RoGuard;
    impl Drop for RoGuard {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }

    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
        .map_err(|error| format!("初始化 Windows OCR 失败: {error}"))?;
    let _guard = RoGuard;
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.to_string_lossy().as_ref()))
        .and_then(|operation| operation.get())
        .map_err(|error| format!("OCR 无法打开图片: {error}"))?;
    let stream = file
        .OpenAsync(FileAccessMode::Read)
        .and_then(|operation| operation.get())
        .map_err(|error| format!("OCR 无法读取图片: {error}"))?;
    let decoder = BitmapDecoder::CreateAsync(&stream)
        .and_then(|operation| operation.get())
        .map_err(|error| format!("OCR 无法解码图片: {error}"))?;
    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .and_then(|operation| operation.get())
        .map_err(|error| format!("OCR 无法取得位图: {error}"))?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|error| format!("系统没有可用的 OCR 语言包: {error}"))?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .and_then(|operation| operation.get())
        .map_err(|error| format!("OCR 识别失败: {error}"))?;
    result
        .Text()
        .map(|text| text.to_string())
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "windows"))]
fn ocr_image(_path: &Path) -> Result<String, String> {
    Err("本地 OCR 仅在 Windows 上可用".into())
}

fn extract_text(path: &Path) -> Result<Option<String>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_TEXT_BYTES * 32 {
        return Ok(None);
    }
    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let value = match extension.as_str() {
        "txt" | "md" | "csv" | "tsv" | "json" | "xml" | "html" | "htm" | "log" | "ini" | "yaml"
        | "yml" => {
            if metadata.len() > MAX_TEXT_BYTES {
                return Ok(None);
            }
            read_text_file(path)?
        }
        "pdf" => extract_pdf(path)?,
        "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" => extract_office(path)?,
        "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tif" | "tiff" => ocr_image(path)?,
        _ => return Ok(None),
    };
    Ok(Some(value.chars().take(MAX_INDEX_CHARS).collect()))
}

pub fn rebuild(app: &AppHandle, pod_id: Option<u64>) -> Result<IndexResult, String> {
    if !policy::allow_index()? {
        return Err("机构策略已禁止全文和 OCR 索引".into());
    }
    let state = app.state::<AppState>();
    let (settings, items) = {
        let connection = state.db.lock().unwrap();
        let settings = staging::load_settings_from(&connection, &state)?;
        let items = match pod_id {
            Some(pod_id) => db::items_of_pod(&connection, pod_id as i64)?,
            None => db::list_items(&connection)?,
        };
        (settings, items)
    };
    let mut result = IndexResult {
        indexed: 0,
        skipped: 0,
        failures: Vec::new(),
        ocr_available: cfg!(target_os = "windows"),
    };
    for item in items {
        let pod = settings
            .pods
            .iter()
            .find(|pod| pod.id == item.pod_id as u64);
        if pod.is_some_and(|pod| {
            pod.security.enabled
                && (pod.security.suppress_index || security::is_locked(app, pod.id))
        }) {
            result.skipped += 1;
            continue;
        }
        let path = match staging::item_path(&item, &settings) {
            Ok(path) => path,
            Err(error) => {
                result.failures.push(format!("{}：{error}", item.name));
                continue;
            }
        };
        match extract_text(&path) {
            Ok(Some(text)) => {
                state
                    .db
                    .lock()
                    .unwrap()
                    .execute(
                        "INSERT INTO item_annotations (item_id, indexed_text, indexed_at)
                         VALUES (?1, ?2, ?3)
                         ON CONFLICT(item_id) DO UPDATE SET
                           indexed_text = excluded.indexed_text,
                           indexed_at = excluded.indexed_at",
                        params![item.id, text, db::now_ms()],
                    )
                    .map_err(|error| error.to_string())?;
                result.indexed += 1;
            }
            Ok(None) => result.skipped += 1,
            Err(error) => result.failures.push(format!("{}：{error}", item.name)),
        }
    }
    Ok(result)
}

pub fn update_annotation(
    app: &AppHandle,
    item_id: i64,
    tags: Vec<String>,
    note: String,
) -> Result<(), String> {
    let mut tags = tags
        .into_iter()
        .map(|tag| tag.trim().trim_start_matches('#').to_string())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    tags.sort_by_key(|tag| tag.to_lowercase());
    tags.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    if tags.len() > 16 || tags.iter().any(|tag| tag.chars().count() > 32) {
        return Err("每个项目最多 16 个标签，每个标签最多 32 个字符".into());
    }
    if note.chars().count() > 2_000 {
        return Err("备注不能超过 2000 个字符".into());
    }
    let state = app.state::<AppState>();
    let item = db::items_by_ids(&state.db.lock().unwrap(), &[item_id])?
        .into_iter()
        .next()
        .ok_or_else(|| "暂存项目不存在".to_string())?;
    security::require_items_unlocked(app, std::slice::from_ref(&item))?;
    state
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO item_annotations (item_id, tags, note)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(item_id) DO UPDATE SET tags = excluded.tags, note = excluded.note",
            params![
                item_id,
                serde_json::to_string(&tags).map_err(|error| error.to_string())?,
                note.trim()
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - (month <= 2) as i64;
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn parse_date(value: &str) -> Option<i64> {
    let mut parts = value.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day).saturating_mul(86_400_000))
}

fn snippet(text: &str, term: Option<&str>) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = text.to_lowercase();
    let start = term
        .and_then(|term| {
            lower
                .find(&term.to_lowercase())
                .map(|byte| lower[..byte].chars().count())
        })
        .map(|characters| characters.saturating_sub(50))
        .unwrap_or(0);
    text.chars().skip(start).take(180).collect()
}

pub fn search(
    app: &AppHandle,
    query: String,
    pod_id: Option<u64>,
) -> Result<Vec<SearchHit>, String> {
    let state = app.state::<AppState>();
    let connection = state.db.lock().unwrap();
    let mut statement = connection
        .prepare(
            "SELECT i.id, i.pod_id, i.kind, i.staging_path, i.original_path, i.name,
                    i.ext, i.size, i.created_at,
                    COALESCE(a.tags, '[]'), COALESCE(a.note, ''), COALESCE(a.indexed_text, '')
             FROM items i LEFT JOIN item_annotations a ON a.item_id = i.id
             WHERE (?1 IS NULL OR i.pod_id = ?1) ORDER BY i.created_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![pod_id.map(|id| id as i64)], |row| {
            Ok((
                StagedItem {
                    id: row.get(0)?,
                    pod_id: row.get(1)?,
                    kind: row.get(2)?,
                    staging_path: row.get(3)?,
                    original_path: row.get(4)?,
                    name: row.get(5)?,
                    ext: row.get(6)?,
                    size: row.get(7)?,
                    created_at: row.get(8)?,
                },
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    drop(connection);

    let mut tags_filter = Vec::new();
    let mut type_filter = None;
    let mut source_filter = None;
    let mut after = None;
    let mut before = None;
    let mut terms = Vec::new();
    for token in query.split_whitespace() {
        if let Some(value) = token.strip_prefix("tag:") {
            tags_filter.push(value.to_lowercase());
        } else if let Some(value) = token.strip_prefix("标签:") {
            tags_filter.push(value.to_lowercase());
        } else if let Some(value) = token.strip_prefix("type:") {
            type_filter = Some(value.trim_start_matches('.').to_lowercase());
        } else if let Some(value) = token.strip_prefix("类型:") {
            type_filter = Some(value.trim_start_matches('.').to_lowercase());
        } else if let Some(value) = token.strip_prefix("source:") {
            source_filter = Some(value.to_lowercase());
        } else if let Some(value) = token.strip_prefix("来源:") {
            source_filter = Some(value.to_lowercase());
        } else if let Some(value) = token.strip_prefix("after:") {
            after = parse_date(value);
        } else if let Some(value) = token.strip_prefix("before:") {
            before = parse_date(value);
        } else if token == "上周" || token == "最近一周" {
            after = Some(db::now_ms().saturating_sub(7 * 86_400_000));
        } else {
            terms.push(token.to_lowercase());
        }
    }

    let mut hits = Vec::new();
    for (item, tags_json, note, indexed_text) in rows {
        if security::is_locked(app, item.pod_id as u64) {
            continue;
        }
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        if !tags_filter
            .iter()
            .all(|wanted| tags.iter().any(|tag| tag.eq_ignore_ascii_case(wanted)))
        {
            continue;
        }
        if type_filter.as_ref().is_some_and(|wanted| {
            item.ext.as_deref().unwrap_or("").to_lowercase() != *wanted
                && item.kind.to_lowercase() != *wanted
        }) {
            continue;
        }
        if source_filter.as_ref().is_some_and(|wanted| {
            !item
                .original_path
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains(wanted)
        }) {
            continue;
        }
        if after.is_some_and(|value| item.created_at < value)
            || before.is_some_and(|value| item.created_at >= value.saturating_add(86_400_000))
        {
            continue;
        }
        let name = item.name.to_lowercase();
        let path = item.staging_path.to_lowercase();
        let source = item
            .original_path
            .clone()
            .unwrap_or_default()
            .to_lowercase();
        let note_lower = note.to_lowercase();
        let text_lower = indexed_text.to_lowercase();
        let tags_lower = tags.join(" ").to_lowercase();
        if !terms.iter().all(|term| {
            name.contains(term)
                || path.contains(term)
                || source.contains(term)
                || note_lower.contains(term)
                || text_lower.contains(term)
                || tags_lower.contains(term)
        }) {
            continue;
        }
        let mut matched_on = Vec::new();
        if terms.iter().any(|term| name.contains(term)) {
            matched_on.push("文件名".into());
        }
        if terms.iter().any(|term| note_lower.contains(term)) {
            matched_on.push("备注".into());
        }
        if terms.iter().any(|term| text_lower.contains(term)) {
            matched_on.push("文件内容/OCR".into());
        }
        hits.push(SearchHit {
            item,
            tags,
            note,
            snippet: snippet(&indexed_text, terms.first().map(String::as_str)),
            matched_on,
        });
    }
    Ok(hits)
}

pub fn annotation(app: &AppHandle, item_id: i64) -> Result<Annotation, String> {
    let state = app.state::<AppState>();
    let item = db::items_by_ids(&state.db.lock().unwrap(), &[item_id])?
        .into_iter()
        .next()
        .ok_or_else(|| "暂存项目不存在".to_string())?;
    security::require_items_unlocked(app, std::slice::from_ref(&item))?;
    let value = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT tags, note FROM item_annotations WHERE item_id = ?1",
            params![item_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((tags, note)) = value else {
        return Ok(Annotation {
            tags: Vec::new(),
            note: String::new(),
        });
    };
    Ok(Annotation {
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_text_extraction_removes_tags() {
        assert_eq!(strip_xml("<p>Hello <b>世界</b></p>").trim(), "Hello  世界");
    }

    #[test]
    fn date_parser_handles_valid_and_invalid_dates() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert!(parse_date("2026-99-01").is_none());
    }

    #[test]
    fn snippet_is_bounded_and_centered_near_match() {
        let value = format!("{} needle {}", "a".repeat(90), "b".repeat(200));
        let value = snippet(&value, Some("needle"));
        assert!(value.contains("needle"));
        assert!(value.chars().count() <= 180);
    }
}
