//! 本地隐私提示与安全副本生成。扫描不会上传文件，也不会修改原件。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufReader, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use exif::{In, Reader as ExifReader, Tag};
use regex::Regex;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::db;
use crate::file_ops;
use crate::rules;
use crate::security;
use crate::staging;
use crate::state::AppState;

const MAX_METADATA_SCAN_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyIssue {
    pub path: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    pub can_clean: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyScanResult {
    pub files_scanned: usize,
    pub issues: Vec<PrivacyIssue>,
    pub duplicates: Vec<Vec<String>>,
    pub disclaimer: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    pub source: String,
    pub output: String,
    pub removed: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeExportResult {
    pub completed: Vec<CleanResult>,
    pub failed: Vec<String>,
}

fn id_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"(?i)(?:\D|^)[1-9]\d{5}(?:18|19|20)\d{2}(?:0[1-9]|1[0-2])(?:0[1-9]|[12]\d|3[01])\d{3}[0-9x](?:\D|$)").unwrap())
}

fn phone_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"(?:\D|^)1[3-9]\d{9}(?:\D|$)").unwrap())
}

fn email_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"(?i)[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}").unwrap())
}

fn push_issue(
    issues: &mut Vec<PrivacyIssue>,
    path: &Path,
    code: &str,
    severity: &str,
    message: impl Into<String>,
    can_clean: bool,
) {
    issues.push(PrivacyIssue {
        path: path.to_string_lossy().to_string(),
        code: code.into(),
        severity: severity.into(),
        message: message.into(),
        can_clean,
    });
}

fn scan_name(path: &Path, issues: &mut Vec<PrivacyIssue>) {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
    if id_pattern().is_match(&name) {
        push_issue(
            issues,
            path,
            "filename-id",
            "high",
            "文件名可能包含身份证号码",
            false,
        );
    }
    if phone_pattern().is_match(&name) {
        push_issue(
            issues,
            path,
            "filename-phone",
            "medium",
            "文件名可能包含手机号码",
            false,
        );
    }
    if email_pattern().is_match(&name) {
        push_issue(
            issues,
            path,
            "filename-email",
            "medium",
            "文件名可能包含电子邮箱",
            false,
        );
    }
    let lower = name.to_lowercase();
    if name.starts_with('.')
        || lower.ends_with(".tmp")
        || lower.ends_with(".bak")
        || lower.ends_with('~')
        || lower.starts_with("~$")
    {
        push_issue(
            issues,
            path,
            "temporary-file",
            "medium",
            "隐藏文件、临时文件或备份文件通常不应交付",
            false,
        );
    }
}

fn scan_exif(path: &Path, issues: &mut Vec<PrivacyIssue>) -> Result<(), String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(file);
    let Ok(exif) = ExifReader::new().read_from_container(&mut reader) else {
        return Ok(());
    };
    let has_gps = exif.get_field(Tag::GPSLatitude, In::PRIMARY).is_some()
        || exif.get_field(Tag::GPSLongitude, In::PRIMARY).is_some();
    if has_gps {
        push_issue(
            issues,
            path,
            "exif-gps",
            "high",
            "图片 EXIF 中包含 GPS 位置信息",
            true,
        );
    }
    let identity_tags = [
        Tag::Artist,
        Tag::Make,
        Tag::Model,
        Tag::Software,
        Tag::Copyright,
    ];
    if identity_tags
        .iter()
        .any(|tag| exif.get_field(*tag, In::PRIMARY).is_some())
    {
        push_issue(
            issues,
            path,
            "exif-identity",
            "medium",
            "图片 EXIF 中包含作者、设备型号、软件或版权信息",
            true,
        );
    }
    Ok(())
}

fn office_metadata<R: Read + Seek>(reader: R) -> Result<Vec<String>, String> {
    let mut archive = ZipArchive::new(reader).map_err(|error| error.to_string())?;
    let mut findings = Vec::new();
    for entry in ["docProps/core.xml", "docProps/app.xml"] {
        let Ok(mut file) = archive.by_name(entry) else {
            continue;
        };
        if file.size() > 2 * 1024 * 1024 {
            continue;
        }
        let mut xml = String::new();
        file.read_to_string(&mut xml)
            .map_err(|error| error.to_string())?;
        for (needle, label) in [
            ("<dc:creator", "作者"),
            ("<cp:lastModifiedBy", "最后编辑者"),
            ("<Company", "公司"),
            ("<Manager", "管理者"),
        ] {
            if xml.contains(needle) {
                findings.push(label.into());
            }
        }
    }
    findings.sort();
    findings.dedup();
    Ok(findings)
}

fn scan_document_metadata(path: &Path, issues: &mut Vec<PrivacyIssue>) -> Result<(), String> {
    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "jpg" | "jpeg" | "tif" | "tiff" | "png" | "webp" => scan_exif(path, issues)?,
        "pdf" => {
            if let Ok(document) = lopdf::Document::load(path) {
                if document.trailer.get(b"Info").is_ok() {
                    push_issue(
                        issues,
                        path,
                        "pdf-metadata",
                        "medium",
                        "PDF 包含标题、作者、创建工具或其他文档属性",
                        true,
                    );
                }
            } else {
                push_issue(
                    issues,
                    path,
                    "damaged-document",
                    "high",
                    "PDF 无法解析，可能已损坏或受密码保护",
                    false,
                );
            }
        }
        "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" => {
            match fs::File::open(path)
                .map_err(|error| error.to_string())
                .and_then(office_metadata)
            {
                Ok(findings) if !findings.is_empty() => push_issue(
                    issues,
                    path,
                    "office-metadata",
                    "medium",
                    format!("文档属性包含：{}", findings.join("、")),
                    true,
                ),
                Err(_) => push_issue(
                    issues,
                    path,
                    "damaged-document",
                    "high",
                    "Office/OpenDocument 文件无法解析，可能已损坏",
                    false,
                ),
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>, issues: &mut Vec<PrivacyIssue>) {
    scan_name(path, issues);
    let Ok(metadata) = fs::symlink_metadata(path) else {
        push_issue(issues, path, "unreadable", "high", "文件无法读取", false);
        return;
    };
    if file_ops::is_reparse_or_symlink(&metadata) {
        push_issue(
            issues,
            path,
            "link",
            "high",
            "符号链接或目录重解析点可能指向交接范围之外",
            false,
        );
        return;
    }
    if metadata.is_file() {
        if metadata.len() == 0 {
            push_issue(issues, path, "zero-byte", "medium", "文件大小为 0", false);
        }
        files.push(path.to_path_buf());
        return;
    }
    if metadata.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            push_issue(issues, path, "unreadable", "high", "目录无法读取", false);
            return;
        };
        let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
        for entry in entries {
            collect_files(&entry.path(), files, issues);
        }
    }
}

pub fn scan_paths(paths: &[PathBuf]) -> PrivacyScanResult {
    let mut issues = Vec::new();
    let mut files = Vec::new();
    for path in paths {
        collect_files(path, &mut files, &mut issues);
    }
    let mut hashes: HashMap<(u64, String), Vec<String>> = HashMap::new();
    for path in &files {
        if fs::metadata(path)
            .map(|metadata| metadata.len() <= MAX_METADATA_SCAN_BYTES)
            .unwrap_or(false)
        {
            if let Err(error) = scan_document_metadata(path, &mut issues) {
                push_issue(
                    &mut issues,
                    path,
                    "metadata-scan-error",
                    "low",
                    format!("无法完成元数据检查：{error}"),
                    false,
                );
            }
        }
        if let Ok(metadata) = fs::metadata(path) {
            if let Ok(hash) = rules::sha256_file(path) {
                hashes
                    .entry((metadata.len(), hash))
                    .or_default()
                    .push(path.to_string_lossy().to_string());
            }
        }
    }
    let duplicates = hashes
        .into_values()
        .filter(|group| group.len() > 1)
        .collect::<Vec<_>>();
    for group in &duplicates {
        for path in group {
            push_issue(
                &mut issues,
                Path::new(path),
                "duplicate",
                "low",
                format!("内容与同批次另外 {} 个文件重复", group.len() - 1),
                false,
            );
        }
    }
    PrivacyScanResult {
        files_scanned: files.len(),
        issues,
        duplicates,
        disclaimer: "本地规则只能提示常见风险，不代表完全匿名，也不构成合规认证。".into(),
    }
}

pub fn scan_items(app: &AppHandle, ids: &[i64]) -> Result<PrivacyScanResult, String> {
    let state = app.state::<AppState>();
    let (settings, items) = {
        let connection = state.db.lock().unwrap();
        (
            staging::load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, ids)?,
        )
    };
    staging::validate_item_pods(&settings, &state, &items)?;
    let paths = items
        .iter()
        .map(|item| staging::item_path(item, &settings))
        .collect::<Result<Vec<_>, _>>()?;
    security::require_items_unlocked(app, &items)?;
    Ok(scan_paths(&paths))
}

fn clean_image(source: &Path, output: &Path) -> Result<Vec<String>, String> {
    let image = image::open(source).map_err(|error| format!("无法解码图片: {error}"))?;
    let format = image::ImageFormat::from_path(source).unwrap_or(image::ImageFormat::Png);
    image
        .save_with_format(output, format)
        .map_err(|error| format!("无法写入清理后的图片: {error}"))?;
    Ok(vec!["EXIF、GPS、作者、设备和软件元数据".into()])
}

fn clean_pdf(source: &Path, output: &Path) -> Result<Vec<String>, String> {
    let mut document = lopdf::Document::load(source).map_err(|error| error.to_string())?;
    let mut removed = Vec::new();
    if document.trailer.remove(b"Info").is_some() {
        removed.push("PDF 文档属性".into());
    }
    let root = document
        .trailer
        .get(b"Root")
        .and_then(lopdf::Object::as_reference)
        .ok();
    if let Some(root) = root {
        if let Ok(catalog) = document.get_dictionary_mut(root) {
            if catalog.remove(b"Metadata").is_some() {
                removed.push("PDF XMP 元数据".into());
            }
        }
    }
    document.save(output).map_err(|error| error.to_string())?;
    Ok(removed)
}

fn scrub_xml(xml: &str) -> (String, Vec<String>) {
    static FIELDS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let fields = FIELDS.get_or_init(|| {
        [
            (r"(?is)<dc:creator\b[^>]*>.*?</dc:creator>", "作者"),
            (
                r"(?is)<cp:lastModifiedBy\b[^>]*>.*?</cp:lastModifiedBy>",
                "最后编辑者",
            ),
            (r"(?is)<Company\b[^>]*>.*?</Company>", "公司"),
            (r"(?is)<Manager\b[^>]*>.*?</Manager>", "管理者"),
        ]
        .into_iter()
        .map(|(pattern, label)| (Regex::new(pattern).unwrap(), label))
        .collect()
    });
    let mut value = xml.to_string();
    let mut removed = Vec::new();
    for (pattern, label) in fields {
        if pattern.is_match(&value) {
            value = pattern.replace_all(&value, "").to_string();
            removed.push((*label).into());
        }
    }
    (value, removed)
}

fn clean_office(source: &Path, output: &Path) -> Result<Vec<String>, String> {
    let input = fs::File::open(source).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(input).map_err(|error| error.to_string())?;
    let output_file = fs::File::create(output).map_err(|error| error.to_string())?;
    let mut writer = ZipWriter::new(output_file);
    let mut removed = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let name = entry.name().replace('\\', "/");
        let options = SimpleFileOptions::default().compression_method(entry.compression());
        if entry.is_dir() {
            writer
                .add_directory(name, options)
                .map_err(|error| error.to_string())?;
            continue;
        }
        writer
            .start_file(name.clone(), options)
            .map_err(|error| error.to_string())?;
        if matches!(name.as_str(), "docProps/core.xml" | "docProps/app.xml") {
            if entry.size() > 2 * 1024 * 1024 {
                return Err(format!("文档属性文件异常过大，拒绝清理: {name}"));
            }
            let mut xml = String::new();
            entry
                .read_to_string(&mut xml)
                .map_err(|error| error.to_string())?;
            let (xml, fields) = scrub_xml(&xml);
            removed.extend(fields);
            writer
                .write_all(xml.as_bytes())
                .map_err(|error| error.to_string())?;
        } else {
            std::io::copy(&mut entry, &mut writer).map_err(|error| error.to_string())?;
        }
    }
    writer.finish().map_err(|error| error.to_string())?;
    removed.sort();
    removed.dedup();
    Ok(removed)
}

pub fn clean_copy(source: &Path, output: &Path) -> Result<CleanResult, String> {
    if !source.is_file() {
        return Err("隐私清理目前只针对单个文件；文件夹会在交接包中逐文件处理".into());
    }
    let parent = output
        .parent()
        .ok_or_else(|| "清理副本目标没有父目录".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    file_ops::ensure_distinct_target(source, output)?;
    let temporary = file_ops::unique_target(
        parent,
        &format!(".floepod-privacy-{}-{}", std::process::id(), db::now_ms()),
        &mut HashSet::new(),
    )?;
    let extension = source
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let cleaned = match extension.as_str() {
        "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tif" | "tiff" => clean_image(source, &temporary),
        "pdf" => clean_pdf(source, &temporary),
        "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" => clean_office(source, &temporary),
        _ => file_ops::copy_path(source, &temporary)
            .map(|_| Vec::new())
            .map_err(|error| error.to_string()),
    };
    let removed = match cleaned {
        Ok(removed) => removed,
        Err(error) => {
            let _ = file_ops::remove_path(&temporary);
            return Err(error);
        }
    };
    if let Err(error) = fs::rename(&temporary, output) {
        let cleanup = file_ops::remove_path(&temporary).err();
        return Err(match cleanup {
            Some(cleanup) => format!("无法发布清理副本: {error}；清理临时文件失败: {cleanup}"),
            None => format!("无法发布清理副本: {error}"),
        });
    }
    Ok(CleanResult {
        source: source.to_string_lossy().to_string(),
        output: output.to_string_lossy().to_string(),
        removed,
        warnings: vec!["已生成清理后的副本，原文件未修改；请在交付前自行复核内容。".into()],
    })
}

pub fn safe_export(
    app: AppHandle,
    ids: Vec<i64>,
    destination: String,
) -> Result<SafeExportResult, String> {
    let state = app.state::<AppState>();
    let _operation = state.file_ops.lock().unwrap();
    let (settings, items) = {
        let connection = state.db.lock().unwrap();
        (
            staging::load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, &ids)?,
        )
    };
    security::require_items_unlocked(&app, &items)?;
    staging::validate_item_pods(&settings, &state, &items)?;
    let destination = PathBuf::from(destination);
    if !destination.is_absolute() {
        return Err("安全导出目标必须是绝对路径".into());
    }
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    let mut reserved = std::collections::HashSet::new();
    let mut completed = Vec::new();
    let mut failed = Vec::new();
    for item in &items {
        let source = staging::item_path(item, &settings)?;
        if source.is_dir() {
            failed.push(format!("{}：请使用可信交接包逐文件清理文件夹", item.name));
            continue;
        }
        let target = match file_ops::unique_target(&destination, &item.name, &mut reserved) {
            Ok(target) => target,
            Err(error) => {
                failed.push(format!("{}：{error}", item.name));
                continue;
            }
        };
        match clean_copy(&source, &target) {
            Ok(result) => completed.push(result),
            Err(error) => failed.push(format!("{}：{error}", item.name)),
        }
    }
    let history_items = completed
        .iter()
        .map(|result| {
            let target = PathBuf::from(&result.output);
            crate::operations::OperationItemDraft {
                item_id: None,
                name: target
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_else(|| result.output.clone()),
                source_path: Some(result.source.clone()),
                target_path: Some(result.output.clone()),
                action: "privacy-clean".into(),
                status: "completed".into(),
                error: None,
                snapshot: None,
                compensation: Some(crate::operations::CompensationDraft {
                    kind: "delete_export_copy".into(),
                    source_path: None,
                    target_path: Some(result.output.clone()),
                    expected_signature: crate::operations::signature(&target).ok(),
                }),
            }
        })
        .collect::<Vec<_>>();
    if !history_items.is_empty() {
        let _ = crate::operations::record(
            &state.db.lock().unwrap(),
            crate::operations::OperationDraft::completed(
                "privacy_export",
                items.first().map(|item| item.pod_id),
                format!(
                    "安全导出 {} 项到 {}",
                    completed.len(),
                    destination.display()
                ),
                serde_json::json!({}),
                history_items,
            ),
        );
    }
    Ok(SafeExportResult { completed, failed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_patterns_report_common_identifiers_without_uploading_content() {
        let result = scan_paths(&[PathBuf::from("张三_11010519900101123X_13800138000.pdf")]);
        let codes = result
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<Vec<_>>();
        assert!(codes.contains(&"filename-id"));
        assert!(codes.contains(&"filename-phone"));
    }

    #[test]
    fn xml_scrubber_removes_author_company_and_editor() {
        let xml = "<x><dc:creator>A</dc:creator><cp:lastModifiedBy>B</cp:lastModifiedBy><Company>C</Company></x>";
        let (cleaned, removed) = scrub_xml(xml);
        assert!(!cleaned.contains('A'));
        assert_eq!(removed.len(), 3);
    }

    #[test]
    fn plain_text_clean_copy_preserves_original_and_content() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source.txt");
        let output = temporary.path().join("clean.txt");
        fs::write(&source, b"hello").unwrap();
        let result = clean_copy(&source, &output).unwrap();
        assert!(result.removed.is_empty());
        assert_eq!(fs::read(&source).unwrap(), b"hello");
        assert_eq!(fs::read(output).unwrap(), b"hello");
    }
}
