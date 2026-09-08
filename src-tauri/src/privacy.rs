//! 本地隐私提示与安全副本生成。扫描不会上传文件，也不会修改原件。

use std::fs;
#[cfg(test)]
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};
#[cfg(test)]
use zip::write::SimpleFileOptions;
#[cfg(test)]
use zip::ZipWriter;

use crate::db;
use crate::file_ops;
use crate::security;
use crate::staging;
use crate::state::AppState;

pub(crate) mod clean;
pub(crate) mod metadata;

mod scan;
pub use scan::scan_paths;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    Checked,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetadataCheck {
    pub path: String,
    pub status: ScanStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyIssue {
    pub path: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    pub can_clean: bool,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyScanResult {
    pub files_scanned: usize,
    pub files_checked: usize,
    pub files_skipped: usize,
    pub files_failed: usize,
    pub files: Vec<MetadataCheck>,
    pub issues: Vec<PrivacyIssue>,
    pub duplicates: Vec<Vec<String>>,
    pub disclaimer: String,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    pub source: String,
    pub output: String,
    pub removed: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SafeExportResult {
    pub completed: Vec<CleanResult>,
    pub failed: Vec<String>,
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

pub fn clean_copy(source: &Path, output: &Path) -> Result<CleanResult, String> {
    if !source.is_file() {
        return Err("隐私清理目前只针对单个文件；文件夹会在交接包中逐文件处理".into());
    }
    let parent = output
        .parent()
        .ok_or_else(|| "清理副本目标没有父目录".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    file_ops::ensure_distinct_target(source, output)?;
    let workspace = tempfile::Builder::new()
        .prefix(".floepod-privacy-")
        .tempdir_in(parent)
        .map_err(|error| format!("无法创建清理临时目录: {error}"))?;
    let temporary = workspace.path().join("cleaned");
    let extension = source
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let cleaned = match extension.as_str() {
        "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tif" | "tiff" | "pdf" | "docx" | "xlsx"
        | "pptx" | "odt" | "ods" | "odp" => crate::parser_worker::invoke::<Vec<String>>(
            crate::parser_worker::Operation::Clean {
                source: source.to_path_buf(),
                output: temporary.clone(),
            },
            Vec::new(),
        )
        .map_err(|error| format!("清理未完成: {error:?}")),
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
    if let Err(error) = file_ops::rename_new(&temporary, output) {
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
    let _permit = state.tasks.enter()?;
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
        if let Err(error) = crate::operations::record(
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
        ) {
            crate::logging::write(&format!(
                "[privacy] 安全副本已生成，但操作记录写入失败: {error}"
            ));
            for result in &mut completed {
                result
                    .warnings
                    .push(format!("副本已生成，但无法建立操作记录与撤销入口: {error}"));
            }
        }
    }
    Ok(SafeExportResult { completed, failed })
}

#[cfg(test)]
mod tests {
    use super::clean::scrub_xml;
    use super::*;

    #[test]
    fn oversized_and_unsupported_metadata_are_skipped_not_clean() {
        let temp = tempfile::tempdir().unwrap();
        let large = temp.path().join("large.jpg");
        fs::File::create(&large)
            .unwrap()
            .set_len(metadata::MAX_INPUT_BYTES + 1)
            .unwrap();
        let text = temp.path().join("note.txt");
        fs::write(&text, "hello").unwrap();
        let result = scan_paths(&[large, text]);
        assert_eq!(result.files_scanned, 2);
        assert_eq!(result.files_checked, 0);
        assert_eq!(result.files_skipped, 2);
        assert!(result.files.iter().all(|file| file.reason.is_some()));
    }

    #[test]
    fn malformed_documents_and_missing_files_have_explicit_failed_status() {
        let temp = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for extension in ["pdf", "jpg", "docx"] {
            let path = temp.path().join(format!("broken.{extension}"));
            fs::write(&path, b"not a document").unwrap();
            paths.push(path);
        }
        paths.push(temp.path().join("missing.pdf"));
        let result = scan_paths(&paths);
        assert_eq!(result.files_failed, 4);
        assert_eq!(result.files_checked, 0);
        assert!(result
            .files
            .iter()
            .all(|file| file.status == ScanStatus::Failed));
    }

    fn office_fixture(path: &Path, entry: &str, xml: &[u8]) {
        let mut archive = ZipWriter::new(fs::File::create(path).unwrap());
        archive
            .start_file(entry, SimpleFileOptions::default())
            .unwrap();
        archive.write_all(xml).unwrap();
        archive.finish().unwrap();
    }

    #[test]
    fn opendocument_metadata_is_detected_and_cleaned_in_a_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.odt");
        office_fixture(&source, "meta.xml", br#"<office:document-meta xmlns:office="urn:office" xmlns:dc="urn:dc" xmlns:meta="urn:meta"><office:meta><dc:creator>Alice</dc:creator><meta:initial-creator>Bob</meta:initial-creator></office:meta></office:document-meta>"#);
        let original = fs::read(&source).unwrap();
        let result = scan_paths(std::slice::from_ref(&source));
        assert_eq!(result.files_checked, 1);
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "office-metadata"));
        let output = temp.path().join("clean.odt");
        clean_copy(&source, &output).unwrap();
        assert_eq!(fs::read(&source).unwrap(), original);
        assert!(!scan_paths(&[output])
            .issues
            .iter()
            .any(|issue| issue.code == "office-metadata"));
    }

    #[test]
    fn oversized_or_invalid_office_properties_cannot_report_checked() {
        let temp = tempfile::tempdir().unwrap();
        let large = temp.path().join("large.docx");
        office_fixture(
            &large,
            "docProps/core.xml",
            &vec![b'x'; metadata::MAX_PROPERTY_BYTES as usize + 1],
        );
        let broken = temp.path().join("broken.docx");
        office_fixture(&broken, "docProps/core.xml", b"<root><creator>");
        let result = scan_paths(&[large, broken]);
        assert_eq!(result.files_skipped, 1);
        assert_eq!(result.files_failed, 1);
        assert_eq!(result.files_checked, 0);
    }

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
        let (cleaned, removed) = scrub_xml(xml).unwrap();
        assert!(!cleaned.contains('A'));
        assert_eq!(removed.len(), 3);
    }

    #[test]
    fn xml_scrubber_handles_aliases_empty_fields_and_rejects_dtd() {
        let (cleaned, removed) = scrub_xml(r#"<root xmlns:alias="urn:dc"><alias:creator>Alice</alias:creator><Company/><Title>Keep</Title></root>"#).unwrap();
        assert!(!cleaned.contains("Alice"));
        assert!(cleaned.contains("Keep"));
        assert!(!removed.is_empty());
        assert!(
            scrub_xml(r#"<!DOCTYPE r [<!ENTITY x SYSTEM "file:///secret">]><r>&x;</r>"#).is_err()
        );
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
