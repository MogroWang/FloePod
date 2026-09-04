//! 可信交接包：本地复制、可选元数据清理、清单、SHA-256 与离线验证说明。

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db;
use crate::file_ops;
use crate::operations::{self, CompensationDraft, OperationDraft, OperationItemDraft};
use crate::privacy;
use crate::rules;
use crate::security;
use crate::settings;
use crate::staging;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffFile {
    pub relative_path: String,
    pub size: u64,
    pub sha256: String,
    pub source: String,
    pub cleaned: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffResult {
    pub directory: String,
    pub files: Vec<HandoffFile>,
    pub missing: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyIssue {
    pub path: String,
    pub expected: String,
    pub actual: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResult {
    pub checked: usize,
    pub valid: usize,
    pub issues: Vec<VerifyIssue>,
}

fn safe_name(value: &str) -> String {
    let invalid = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let value = value
        .chars()
        .map(|character| {
            if character.is_control() || invalid.contains(&character) {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let value = value.trim().trim_matches('.').trim();
    if value.is_empty() {
        "FloePod交接包".into()
    } else {
        value.chars().take(80).collect()
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn csv(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn copy_file(
    source: &Path,
    target: &Path,
    root: &Path,
    clean_metadata: bool,
    files: &mut Vec<HandoffFile>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let mut cleaned = Vec::new();
    if clean_metadata {
        match privacy::clean_copy(source, target) {
            Ok(result) => cleaned = result.removed,
            Err(error) => {
                return Err(format!(
                    "{} 隐私清理失败，未把原件放入交接包: {error}",
                    source.display()
                ));
            }
        }
    } else {
        file_ops::copy_path(source, target).map_err(|error| error.to_string())?;
    }
    let size = fs::metadata(target)
        .map_err(|error| error.to_string())?
        .len();
    let sha256 = rules::sha256_file(target)?;
    let relative_path = target
        .strip_prefix(root)
        .unwrap_or(target)
        .to_string_lossy()
        .replace('\\', "/");
    if clean_metadata && cleaned.is_empty() {
        warnings.push(format!(
            "{} 没有可自动清理的已知元数据；内容仍需人工复核",
            relative_path
        ));
    }
    files.push(HandoffFile {
        relative_path,
        size,
        sha256,
        source: source.to_string_lossy().to_string(),
        cleaned,
    });
    Ok(())
}

fn copy_tree(
    source: &Path,
    target: &Path,
    root: &Path,
    clean_metadata: bool,
    files: &mut Vec<HandoffFile>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if file_ops::is_reparse_or_symlink(&metadata) {
        return Err(format!(
            "交接包拒绝符号链接或目录重解析点: {}",
            source.display()
        ));
    }
    if metadata.is_file() {
        return copy_file(source, target, root, clean_metadata, files, warnings);
    }
    if !metadata.is_dir() {
        return Err(format!("不支持的文件类型: {}", source.display()));
    }
    fs::create_dir_all(target).map_err(|error| error.to_string())?;
    let mut entries = fs::read_dir(source)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
    for entry in entries {
        copy_tree(
            &entry.path(),
            &target.join(entry.file_name()),
            root,
            clean_metadata,
            files,
            warnings,
        )?;
    }
    Ok(())
}

fn write_manifests(
    directory: &Path,
    title: &str,
    note: &str,
    files: &[HandoffFile],
    missing: &[String],
    warnings: &[String],
) -> Result<(), String> {
    let mut sums =
        fs::File::create(directory.join("SHA256SUMS.txt")).map_err(|error| error.to_string())?;
    for file in files {
        writeln!(sums, "{}  {}", file.sha256, file.relative_path)
            .map_err(|error| error.to_string())?;
    }

    let mut manifest =
        fs::File::create(directory.join("文件清单.csv")).map_err(|error| error.to_string())?;
    writeln!(manifest, "相对路径,字节数,SHA-256,原位置,已清理内容")
        .map_err(|error| error.to_string())?;
    for file in files {
        writeln!(
            manifest,
            "{},{},{},{},{}",
            csv(&file.relative_path),
            file.size,
            csv(&file.sha256),
            csv(&file.source),
            csv(&file.cleaned.join("；")),
        )
        .map_err(|error| error.to_string())?;
    }

    let rows = files
        .iter()
        .map(|file| {
            format!(
                "<tr><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
                escape_html(&file.relative_path),
                file.size,
                file.sha256,
                escape_html(&file.cleaned.join("、")),
            )
        })
        .collect::<String>();
    let list = |values: &[String]| {
        if values.is_empty() {
            "<li>无</li>".into()
        } else {
            values
                .iter()
                .map(|value| format!("<li>{}</li>", escape_html(value)))
                .collect::<String>()
        }
    };
    let html = format!(
        r#"<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>{title}</title><style>body{{font:15px/1.6 system-ui;margin:40px;max-width:1100px}}table{{border-collapse:collapse;width:100%}}th,td{{border:1px solid #bbb;padding:7px;text-align:left;vertical-align:top}}code{{font-size:11px;word-break:break-all}}.note{{padding:12px;background:#eef3f8;border-radius:8px}}</style><h1>{title}</h1><p class="note">{note}</p><p>文件数：{count}</p><table><thead><tr><th>文件</th><th>大小</th><th>SHA-256</th><th>清理说明</th></tr></thead><tbody>{rows}</tbody></table><h2>缺失项</h2><ul>{missing}</ul><h2>提醒</h2><ul>{warnings}</ul><h2>如何验证</h2><p>在 Windows PowerShell 中进入本目录，对照 <code>SHA256SUMS.txt</code> 运行 <code>Get-FileHash -Algorithm SHA256 -LiteralPath "文件名"</code>。哈希完全相同表示文件内容与生成交接包时一致。</p><p>本清单只证明文件内容的一致性，不代表身份、法律效力或行业合规认证。</p></html>"#,
        title = escape_html(title),
        note = escape_html(note),
        count = files.len(),
        rows = rows,
        missing = list(missing),
        warnings = list(warnings),
    );
    fs::write(directory.join("交接说明.html"), html).map_err(|error| error.to_string())?;
    let json = serde_json::to_vec_pretty(&serde_json::json!({
        "title": title,
        "note": note,
        "createdAt": db::now_ms(),
        "files": files,
        "missing": missing,
        "warnings": warnings,
        "disclaimer": "本地清单用于完整性核对，不代表身份、法律效力或合规认证。"
    }))
    .map_err(|error| error.to_string())?;
    fs::write(directory.join("handoff.json"), json).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn create(
    app: AppHandle,
    ids: Vec<i64>,
    destination: String,
    title: String,
    note: String,
    clean_metadata: bool,
) -> Result<HandoffResult, String> {
    let state = app.state::<AppState>();
    let _operation = state.file_ops.lock().unwrap();
    let (current, items) = {
        let connection = state.db.lock().unwrap();
        (
            staging::load_settings_from(&connection, &state)?,
            db::items_by_ids(&connection, &ids)?,
        )
    };
    if items.is_empty() {
        return Err("没有可交接的项目".into());
    }
    security::require_items_unlocked(&app, &items)?;
    settings::validate(&current, &staging::data_dir(&state))?;
    staging::validate_item_pods(&current, &state, &items)?;
    let destination = PathBuf::from(destination);
    if !destination.is_absolute() {
        return Err("交接包目标必须是绝对路径".into());
    }
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    let title = safe_name(&title);
    let folder_name = format!("{}_{}", title, db::now_ms());
    let directory = file_ops::unique_target(&destination, &folder_name, &mut HashSet::new())?;
    fs::create_dir(&directory).map_err(|error| error.to_string())?;

    let build = (|| -> Result<HandoffResult, String> {
        let mut files = Vec::new();
        let mut missing = Vec::new();
        let mut warnings = Vec::new();
        let mut reserved = HashSet::new();
        for item in &items {
            let source = staging::item_path(item, &current)?;
            if !source.exists() {
                missing.push(item.name.clone());
                continue;
            }
            let source_name = source
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .ok_or_else(|| format!("交接源路径没有文件名: {}", source.display()))?;
            let target = file_ops::unique_target(&directory, &source_name, &mut reserved)?;
            if let Err(error) = copy_tree(
                &source,
                &target,
                &directory,
                clean_metadata,
                &mut files,
                &mut warnings,
            ) {
                missing.push(format!("{}：{}", item.name, error));
            }
        }
        if files.is_empty() {
            return Err("交接包没有成功写入任何文件".into());
        }
        write_manifests(&directory, &title, &note, &files, &missing, &warnings)?;
        Ok(HandoffResult {
            directory: directory.to_string_lossy().to_string(),
            files,
            missing,
            warnings,
        })
    })();
    let result = match build {
        Ok(result) => result,
        Err(error) => {
            let _ = file_ops::remove_path(&directory);
            return Err(error);
        }
    };
    let operation_item = OperationItemDraft {
        item_id: None,
        name: title.clone(),
        source_path: None,
        target_path: Some(result.directory.clone()),
        action: "handoff".into(),
        status: if result.missing.is_empty() {
            "completed".into()
        } else {
            "partial".into()
        },
        error: (!result.missing.is_empty()).then(|| result.missing.join("；")),
        snapshot: None,
        compensation: Some(CompensationDraft {
            kind: "delete_export_copy".into(),
            source_path: None,
            target_path: Some(result.directory.clone()),
            expected_signature: operations::signature(&directory).ok(),
        }),
    };
    let _ = operations::record(
        &state.db.lock().unwrap(),
        OperationDraft::completed(
            "handoff",
            items.first().map(|item| item.pod_id),
            format!("生成交接包「{}」（{} 个文件）", title, result.files.len()),
            serde_json::json!({}),
            vec![operation_item],
        ),
    );
    Ok(result)
}

pub fn verify(directory: String) -> Result<VerifyResult, String> {
    let directory = PathBuf::from(directory);
    if !directory.is_absolute() || !directory.is_dir() {
        return Err("请选择有效的交接包目录".into());
    }
    let sums = fs::read_to_string(directory.join("SHA256SUMS.txt"))
        .map_err(|error| format!("无法读取 SHA256SUMS.txt: {error}"))?;
    let mut result = VerifyResult {
        checked: 0,
        valid: 0,
        issues: Vec::new(),
    };
    for line in sums.lines().filter(|line| !line.trim().is_empty()) {
        let Some((expected, relative)) = line.split_once("  ") else {
            continue;
        };
        result.checked += 1;
        let path = match manifest_path(&directory, relative) {
            Ok(path) => path,
            Err(_) => {
                result.issues.push(VerifyIssue {
                    path: relative.into(),
                    expected: expected.into(),
                    actual: None,
                });
                continue;
            }
        };
        match rules::sha256_file(&path) {
            Ok(actual) if actual.eq_ignore_ascii_case(expected) => result.valid += 1,
            Ok(actual) => result.issues.push(VerifyIssue {
                path: relative.into(),
                expected: expected.into(),
                actual: Some(actual),
            }),
            Err(_) => result.issues.push(VerifyIssue {
                path: relative.into(),
                expected: expected.into(),
                actual: None,
            }),
        }
    }
    Ok(result)
}

fn manifest_path(directory: &Path, relative: &str) -> Result<PathBuf, String> {
    let normalized = relative.replace('/', "\\");
    let relative = Path::new(&normalized);
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err("校验清单包含绝对或空路径".into());
    }
    let mut path = directory.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err("校验清单路径越出交接包".into());
        };
        path.push(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if file_ops::is_reparse_or_symlink(&metadata) => {
                return Err("校验清单路径包含符号链接或目录重解析点".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_and_html_escape_untrusted_names() {
        assert_eq!(csv("a,\"b"), "\"a,\"\"b\"");
        assert_eq!(escape_html("<a&b>"), "&lt;a&amp;b&gt;");
    }

    #[test]
    fn verify_detects_changed_and_missing_files() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(temporary.path().join("a.txt"), b"abc").unwrap();
        fs::write(
            temporary.path().join("SHA256SUMS.txt"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  a.txt\n0000  missing.txt\n",
        )
        .unwrap();
        let result = verify(temporary.path().to_string_lossy().to_string()).unwrap();
        assert_eq!(result.checked, 2);
        assert_eq!(result.valid, 1);
        assert_eq!(result.issues.len(), 1);
    }

    #[test]
    fn manifest_paths_cannot_escape_or_follow_links() {
        let temporary = tempfile::tempdir().unwrap();
        assert!(manifest_path(temporary.path(), "../outside.txt").is_err());
        assert!(manifest_path(temporary.path(), r"C:\\outside.txt").is_err());
        assert!(manifest_path(temporary.path(), "/outside.txt").is_err());
        assert_eq!(
            manifest_path(temporary.path(), "folder/file.txt").unwrap(),
            temporary.path().join("folder").join("file.txt")
        );
    }
}
