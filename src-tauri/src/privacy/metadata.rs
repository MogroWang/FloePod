//! Bounded metadata readers. A parser limit or error is never a clean result.
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

use exif::{In, Reader as ExifReader, Tag};
use quick_xml::events::Event;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use super::{push_issue, MetadataCheck, PrivacyIssue, ScanStatus};

pub const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_PROPERTY_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum ScanError {
    Skipped(String),
    Failed(String),
}

fn failed(error: impl std::fmt::Display) -> ScanError {
    ScanError::Failed(error.to_string())
}

fn bounded_bytes(reader: impl Read, limit: u64) -> Result<Vec<u8>, ScanError> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(failed)?;
    if bytes.len() as u64 > limit {
        return Err(ScanError::Skipped(format!(
            "超出解析大小限制（{limit} 字节）"
        )));
    }
    Ok(bytes)
}

fn exif_metadata(
    bytes: &[u8],
    path: &Path,
    issues: &mut Vec<PrivacyIssue>,
) -> Result<(), ScanError> {
    let exif = match ExifReader::new().read_from_container(&mut Cursor::new(bytes)) {
        Ok(exif) => exif,
        Err(exif::Error::NotFound(_)) => return Ok(()),
        Err(error) => return Err(failed(error)),
    };
    if [Tag::GPSLatitude, Tag::GPSLongitude]
        .iter()
        .any(|tag| exif.get_field(*tag, In::PRIMARY).is_some())
    {
        push_issue(
            issues,
            path,
            "exif-gps",
            "high",
            "图片 EXIF 中包含 GPS 位置信息",
            true,
        );
    }
    if [
        Tag::Artist,
        Tag::Make,
        Tag::Model,
        Tag::Software,
        Tag::Copyright,
    ]
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

pub(super) fn xml_properties(bytes: &[u8]) -> Result<Vec<String>, ScanError> {
    let xml = std::str::from_utf8(bytes).map_err(failed)?;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut findings = Vec::new();
    let mut depth = 0usize;
    let mut roots = 0usize;
    loop {
        let event = reader.read_event().map_err(failed)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                if depth == 0 {
                    roots += 1;
                }
                let name = element.local_name();
                let label = property_label(name.as_ref());
                if let Some(label) = label {
                    findings.push(label.into());
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
                if depth > 128 {
                    return Err(ScanError::Skipped("文档属性 XML 嵌套过深".into()));
                }
            }
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| failed("XML 结束标签无对应起点"))?
            }
            Event::DocType(_) => {
                return Err(ScanError::Skipped("文档属性含 DTD，未展开实体".into()))
            }
            Event::Text(ref text)
                if depth == 0 && text.iter().any(|byte| !byte.is_ascii_whitespace()) =>
            {
                return Err(failed("XML 根节点之外含有文本"))
            }
            Event::CData(_) if depth == 0 => return Err(failed("XML 根节点之外含有 CDATA")),
            Event::Eof => break,
            _ => {}
        }
    }
    if roots != 1 || depth != 0 {
        return Err(failed("文档属性 XML 根节点不完整"));
    }
    findings.sort();
    findings.dedup();
    Ok(findings)
}

fn office_metadata(bytes: &[u8]) -> Result<Vec<String>, ScanError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(failed)?;
    if archive.len() > 10_000 {
        return Err(ScanError::Skipped("压缩包条目数量超过检查上限".into()));
    }
    let mut findings = Vec::new();
    for name in ["docProps/core.xml", "docProps/app.xml", "meta.xml"] {
        let file = match archive.by_name(name) {
            Ok(file) => file,
            Err(zip::result::ZipError::FileNotFound) => continue,
            Err(error) => return Err(failed(error)),
        };
        if file.size() > MAX_PROPERTY_BYTES {
            return Err(ScanError::Skipped(format!(
                "文档属性 {name} 超过 2 MiB 限制"
            )));
        }
        findings.extend(xml_properties(&bounded_bytes(file, MAX_PROPERTY_BYTES)?)?);
    }
    findings.sort();
    findings.dedup();
    Ok(findings)
}

fn pdf_metadata(
    bytes: &[u8],
    path: &Path,
    issues: &mut Vec<PrivacyIssue>,
) -> Result<(), ScanError> {
    let document = lopdf::Document::load_mem_with_options(
        bytes,
        lopdf::LoadOptions {
            strict: true,
            max_decompressed_size: Some(MAX_PROPERTY_BYTES as usize),
            ..Default::default()
        },
    )
    .map_err(|error| match error {
        lopdf::Error::Decompress(lopdf::DecompressError::MemoryLimitExceeded { .. }) => {
            ScanError::Skipped("PDF 解压数据超过安全限制".into())
        }
        error => failed(error),
    })?;
    if document.is_encrypted() {
        return Err(ScanError::Skipped(
            "PDF 受密码保护，未完成元数据检查".into(),
        ));
    }
    let xmp = document
        .trailer
        .get(b"Root")
        .and_then(lopdf::Object::as_reference)
        .and_then(|root| document.get_dictionary(root))
        .is_ok_and(|root| root.get(b"Metadata").is_ok());
    if document.trailer.get(b"Info").is_ok() || xmp {
        push_issue(
            issues,
            path,
            "pdf-metadata",
            "medium",
            "PDF 包含文档属性或 XMP 元数据",
            true,
        );
    }
    Ok(())
}

pub(crate) fn inspect(
    bytes: &[u8],
    path: &Path,
    issues: &mut Vec<PrivacyIssue>,
) -> Result<(), ScanError> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "jpg"
            | "jpeg"
            | "tif"
            | "tiff"
            | "png"
            | "webp"
            | "pdf"
            | "docx"
            | "xlsx"
            | "pptx"
            | "odt"
            | "ods"
            | "odp"
    ) {
        return Err(ScanError::Skipped(
            "当前格式不支持元数据检查；已保留文件名和交付风险检查".into(),
        ));
    }
    match extension.as_str() {
        "jpg" | "jpeg" | "tif" | "tiff" | "png" | "webp" => exif_metadata(bytes, path, issues),
        "pdf" => pdf_metadata(bytes, path, issues),
        _ => {
            let findings = office_metadata(bytes)?;
            if !findings.is_empty() {
                push_issue(
                    issues,
                    path,
                    "office-metadata",
                    "medium",
                    format!("文档属性包含：{}", findings.join("、")),
                    true,
                );
            }
            Ok(())
        }
    }
}

pub fn scan(
    path: &Path,
    issues: &mut Vec<PrivacyIssue>,
    bytes_left: &mut u64,
    timeout: std::time::Duration,
) -> (MetadataCheck, Option<(u64, String)>) {
    let mut fingerprint = None;
    let result = (|| {
        if *bytes_left == 0 {
            return Err(ScanError::Skipped(
                "整批读取达到 512 MiB 检查上限，未检查此文件".into(),
            ));
        }
        let file = fs::File::open(path).map_err(failed)?;
        let limit = MAX_INPUT_BYTES.min(*bytes_left);
        if file.metadata().map_err(failed)?.len() > limit {
            return Err(ScanError::Skipped(
                "文件大小超过单文件或整批检查上限".into(),
            ));
        }
        let bytes = match bounded_bytes(file, limit) {
            Ok(bytes) => bytes,
            Err(error) => {
                *bytes_left = bytes_left.saturating_sub(limit);
                return Err(error);
            }
        };
        *bytes_left -= bytes.len() as u64;
        fingerprint = Some((bytes.len() as u64, format!("{:x}", Sha256::digest(&bytes))));
        let found: Vec<PrivacyIssue> = crate::parser_worker::invoke_with_timeout(
            crate::parser_worker::Operation::Inspect {
                display_path: path.to_path_buf(),
            },
            bytes,
            timeout.min(std::time::Duration::from_secs(15)),
        )?;
        issues.extend(found);
        Ok(())
    })();
    let (status, reason) = match result {
        Ok(()) => (ScanStatus::Checked, None),
        Err(ScanError::Skipped(reason)) => (ScanStatus::Skipped, Some(reason)),
        Err(ScanError::Failed(reason)) => (ScanStatus::Failed, Some(reason)),
    };
    (
        MetadataCheck {
            path: path.to_string_lossy().into(),
            status,
            reason,
        },
        fingerprint,
    )
}

pub(super) fn property_label(name: &[u8]) -> Option<&'static str> {
    match name {
        b"creator" | b"initial-creator" => Some("作者"),
        b"lastModifiedBy" | b"printed-by" => Some("最后编辑者"),
        b"Company" => Some("公司"),
        b"Manager" => Some("管理者"),
        b"generator" => Some("创建工具"),
        _ => None,
    }
}
