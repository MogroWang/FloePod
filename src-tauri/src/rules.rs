//! 可解释、无脚本的匣规则：过滤、重命名、日期子目录、重复检测与校验文件。

use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::settings::{self, Pod};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleTarget {
    pub directory: PathBuf,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DateParts {
    year: i32,
    month: u32,
    day: u32,
}

fn today() -> DateParts {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    civil_from_days(days)
}

// Howard Hinnant 的公历换算算法；输入是自 1970-01-01 起的 UTC 日数。
fn civil_from_days(days_since_epoch: i64) -> DateParts {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += (month <= 2) as i64;
    DateParts {
        year: year as i32,
        month: month as u32,
        day: day as u32,
    }
}

fn replace_tokens(pattern: &str, source: &Path, date: DateParts) -> String {
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "未命名".into());
    let stem = source
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| name.clone());
    let extension = source
        .extension()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
    pattern
        .replace("{name}", &name)
        .replace("{stem}", &stem)
        .replace("{ext}", &extension)
        .replace(
            "{date}",
            &format!("{:04}-{:02}-{:02}", date.year, date.month, date.day),
        )
        .replace("{year}", &format!("{:04}", date.year))
        .replace("{month}", &format!("{:02}", date.month))
        .replace("{day}", &format!("{:02}", date.day))
}

fn sanitize_component(value: &str, fallback: &str) -> String {
    let invalid = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let cleaned = value
        .chars()
        .map(|character| {
            if invalid.contains(&character) || character.is_control() {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        fallback.into()
    } else {
        cleaned.chars().take(180).collect()
    }
}

pub fn validate_source(pod: &Pod, source: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    if !pod.rules.enabled {
        return Ok(());
    }
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
    if !pod.rules.name_contains.trim().is_empty()
        && !name
            .to_lowercase()
            .contains(&pod.rules.name_contains.trim().to_lowercase())
    {
        return Err(format!("规则拒绝「{name}」：文件名不包含指定文字"));
    }
    if !pod.rules.allowed_extensions.is_empty() {
        let extension = source
            .extension()
            .map(|value| value.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let allowed = pod
            .rules
            .allowed_extensions
            .iter()
            .map(|value| value.trim().trim_start_matches('.').to_lowercase())
            .any(|value| value == extension);
        if metadata.is_dir() || !allowed {
            return Err(format!("规则拒绝「{name}」：文件类型不在允许列表中"));
        }
    }
    if pod.rules.max_size_mb > 0
        && !metadata.is_dir()
        && metadata.len() > pod.rules.max_size_mb.saturating_mul(1024 * 1024)
    {
        return Err(format!(
            "规则拒绝「{name}」：文件超过 {} MB",
            pod.rules.max_size_mb
        ));
    }
    if !pod.rules.source_folder.trim().is_empty() {
        let allowed = settings::resolve_path(Path::new(&pod.rules.source_folder))?;
        let source = settings::resolve_path(source)?;
        if !settings::path_is_within(&source, &allowed) {
            return Err(format!("规则拒绝「{name}」：文件不在指定来源文件夹中"));
        }
    }
    Ok(())
}

pub fn target(root: &Path, source: &Path, pod: &Pod) -> Result<RuleTarget, String> {
    if !pod.rules.enabled {
        return Ok(RuleTarget {
            directory: root.to_path_buf(),
            name: source
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "未命名".into()),
        });
    }
    let date = today();
    let mut directory = root.to_path_buf();
    let rendered_folder = replace_tokens(&pod.rules.subfolder_pattern, source, date);
    if !rendered_folder.trim().is_empty() {
        let relative = Path::new(&rendered_folder);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err("规则生成了不安全的子目录".into());
        }
        for component in relative.components() {
            if let Component::Normal(value) = component {
                directory.push(sanitize_component(&value.to_string_lossy(), "归档"));
            }
        }
    }
    let pattern = if pod.rules.rename_pattern.trim().is_empty() {
        "{name}"
    } else {
        &pod.rules.rename_pattern
    };
    let rendered_name = replace_tokens(pattern, source, date);
    let mut name = sanitize_component(&rendered_name, "未命名");
    if source.is_file() && Path::new(&name).extension().is_none() {
        if let Some(extension) = source.extension() {
            name.push('.');
            name.push_str(&extension.to_string_lossy());
        }
    }
    Ok(RuleTarget { directory, name })
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("无法读取校验文件 {}: {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn write_checksum_sidecar(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let hash = sha256_file(path)?;
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let sidecar = path.with_file_name(format!("{name}.sha256"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&sidecar)
        .map_err(|error| format!("无法创建校验文件 {}: {error}", sidecar.display()))?;
    writeln!(file, "{hash}  {name}").map_err(|error| error.to_string())?;
    Ok(Some(sidecar))
}

pub fn duplicate_of(source: &Path, existing: &[PathBuf]) -> Result<Option<PathBuf>, String> {
    if !source.is_file() {
        return Ok(None);
    }
    let metadata = fs::metadata(source).map_err(|error| error.to_string())?;
    let source_hash = sha256_file(source)?;
    for candidate in existing {
        let Ok(candidate_metadata) = fs::metadata(candidate) else {
            continue;
        };
        if !candidate_metadata.is_file() || candidate_metadata.len() != metadata.len() {
            continue;
        }
        if sha256_file(candidate)? == source_hash {
            return Ok(Some(candidate.clone()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_date_conversion_covers_epoch_and_leap_day() {
        assert_eq!(
            civil_from_days(0),
            DateParts {
                year: 1970,
                month: 1,
                day: 1
            }
        );
        assert_eq!(
            civil_from_days(19_782),
            DateParts {
                year: 2024,
                month: 2,
                day: 29,
            }
        );
    }

    #[test]
    fn target_renders_safe_date_subfolder_and_preserves_extension() {
        let temporary = tempfile::tempdir().unwrap();
        let mut pod = Pod::default();
        pod.rules.enabled = true;
        pod.rules.rename_pattern = "{date}_{stem}".into();
        pod.rules.subfolder_pattern = "{year}/{month}".into();
        let source = temporary.path().join("合同.pdf");
        fs::write(&source, b"pdf").unwrap();
        let target = target(temporary.path(), &source, &pod).unwrap();
        assert_eq!(
            target.directory.components().count(),
            temporary.path().components().count() + 2
        );
        assert!(target.name.ends_with("_合同.pdf"));
    }

    #[test]
    fn duplicate_detection_uses_content_not_only_name() {
        let temporary = tempfile::tempdir().unwrap();
        let first = temporary.path().join("first.bin");
        let second = temporary.path().join("second.bin");
        fs::write(&first, b"same").unwrap();
        fs::write(&second, b"same").unwrap();
        assert_eq!(
            duplicate_of(&first, std::slice::from_ref(&second)).unwrap(),
            Some(second)
        );
    }

    #[test]
    fn sha256_matches_known_vector() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("a.txt");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
