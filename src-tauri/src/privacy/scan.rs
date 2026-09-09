//! Bounded traversal and batch accounting; filename checks survive unsupported metadata formats.
use super::{metadata, push_issue, MetadataCheck, PrivacyIssue, PrivacyScanResult, ScanStatus};
use crate::file_ops;
use regex::Regex;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

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

fn skipped(checks: &mut Vec<MetadataCheck>, path: &Path, reason: &str) {
    checks.push(MetadataCheck {
        path: path.to_string_lossy().into(),
        status: ScanStatus::Skipped,
        reason: Some(reason.into()),
    });
}

/// Bound all queued entries, including directories and read failures, rather than only regular files.
fn collect_files(
    paths: &[PathBuf],
    limit: usize,
    files: &mut Vec<PathBuf>,
    issues: &mut Vec<PrivacyIssue>,
    checks: &mut Vec<MetadataCheck>,
) {
    let mut pending: Vec<_> = paths
        .iter()
        .take(limit)
        .rev()
        .map(|path| (path.clone(), 0usize))
        .collect();
    let mut capacity = limit.saturating_sub(pending.len());
    if paths.len() > limit {
        skipped(
            checks,
            &paths[limit],
            "选择条目超过检查上限，后续条目未检查",
        );
    }
    while let Some((path, depth)) = pending.pop() {
        if depth > 64 {
            skipped(checks, &path, "目录深度超过检查上限");
            continue;
        }
        scan_name(&path, issues);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                checks.push(MetadataCheck {
                    path: path.to_string_lossy().into(),
                    status: ScanStatus::Failed,
                    reason: Some(error.to_string()),
                });
                push_issue(
                    issues,
                    &path,
                    "unreadable",
                    "high",
                    "文件或目录无法读取",
                    false,
                );
                continue;
            }
        };
        if file_ops::is_reparse_or_symlink(&metadata) {
            skipped(checks, &path, "未跟随符号链接或重解析点");
            push_issue(
                issues,
                &path,
                "link",
                "high",
                "符号链接或目录重解析点可能指向交接范围之外",
                false,
            );
        } else if metadata.is_file() {
            if metadata.len() == 0 {
                push_issue(issues, &path, "zero-byte", "medium", "文件大小为 0", false);
            }
            files.push(path);
        } else if metadata.is_dir() {
            let entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) => {
                    checks.push(MetadataCheck {
                        path: path.to_string_lossy().into(),
                        status: ScanStatus::Failed,
                        reason: Some(error.to_string()),
                    });
                    continue;
                }
            };
            let mut children = Vec::new();
            for entry in entries.take(capacity + 1) {
                if capacity == 0 {
                    skipped(checks, &path, "整批目录条目达到检查上限，剩余内容未检查");
                    break;
                }
                capacity -= 1;
                match entry {
                    Ok(entry) => children.push(entry.path()),
                    Err(error) => checks.push(MetadataCheck {
                        path: path.to_string_lossy().into(),
                        status: ScanStatus::Failed,
                        reason: Some(error.to_string()),
                    }),
                }
            }
            children.sort_by_cached_key(|path| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase()
            });
            pending.extend(children.into_iter().rev().map(|path| (path, depth + 1)));
        } else {
            skipped(checks, &path, "当前文件类型不支持检查");
        }
    }
}
pub fn scan_paths(paths: &[PathBuf]) -> PrivacyScanResult {
    scan_with_limits(
        paths,
        10_000,
        512 * 1024 * 1024,
        std::time::Duration::from_secs(60),
    )
}

fn scan_with_limits(
    paths: &[PathBuf],
    entry_limit: usize,
    mut bytes_left: u64,
    timeout: std::time::Duration,
) -> PrivacyScanResult {
    let started = std::time::Instant::now();
    let mut issues = Vec::new();
    let mut files = Vec::new();
    let mut checks = Vec::new();
    collect_files(paths, entry_limit, &mut files, &mut issues, &mut checks);
    let mut hashes: HashMap<(u64, String), Vec<String>> = HashMap::new();
    for path in &files {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            skipped(&mut checks, path, "整批检查超过时间限制，未检查此文件");
            continue;
        }
        let (check, fingerprint) = metadata::scan(path, &mut issues, &mut bytes_left, remaining);
        checks.push(check);
        if let Some(fingerprint) = fingerprint {
            hashes
                .entry(fingerprint)
                .or_default()
                .push(path.to_string_lossy().into());
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
        files_checked: checks
            .iter()
            .filter(|check| check.status == ScanStatus::Checked)
            .count(),
        files_skipped: checks
            .iter()
            .filter(|check| check.status == ScanStatus::Skipped)
            .count(),
        files_failed: checks
            .iter()
            .filter(|check| check.status == ScanStatus::Failed)
            .count(),
        files: checks,
        issues,
        duplicates,
        disclaimer: "本地规则只能提示常见风险，不代表完全匿名，也不构成合规认证。".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exhausted_deadline_marks_unparsed_files_incomplete() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("document.pdf");
        fs::write(&path, b"unparsed").unwrap();
        let result = scan_with_limits(&[path], 10, 1024, std::time::Duration::ZERO);
        assert_eq!(result.files_checked, 0);
        assert!(result.files_skipped > 0);
    }
    #[test]
    fn directory_budget_counts_directories_and_reports_unvisited_content() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..8 {
            fs::create_dir(root.path().join(index.to_string())).unwrap();
        }
        let result = scan_with_limits(
            &[root.path().to_path_buf()],
            3,
            1024,
            std::time::Duration::from_secs(60),
        );
        assert_eq!(result.files_scanned, 0);
        assert!(result.files_skipped > 0);
        assert!(result.files.len() <= 3);
    }
    #[test]
    fn batch_budget_reports_incomplete_without_reading_the_remaining_files() {
        let root = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..3)
            .map(|i| {
                let path = root.path().join(format!("{i}.txt"));
                fs::write(&path, b"duplicate").unwrap();
                path
            })
            .collect();
        let result = scan_with_limits(&paths, 10, 18, std::time::Duration::from_secs(60));
        assert_eq!(result.files_scanned, 3);
        assert_eq!(result.duplicates.len(), 1);
        assert_eq!(result.duplicates[0].len(), 2);
        assert!(result.files[2].reason.as_ref().unwrap().contains("上限"));
    }
}
