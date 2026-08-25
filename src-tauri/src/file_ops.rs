use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{db, settings};

static OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub fn extension(name: &str) -> Option<String> {
    let index = name.rfind('.')?;
    if index == 0 {
        return None;
    }
    Some(name[index + 1..].to_ascii_lowercase())
}

pub fn unique_target(
    directory: &Path,
    desired_name: &str,
    reserved: &mut HashSet<String>,
) -> Result<PathBuf, String> {
    let mut name = desired_name.to_string();
    let mut suffix = 1;
    loop {
        let candidate = directory.join(&name);
        let key = settings::path_key(&settings::resolve_path(&candidate)?);
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound && !reserved.contains(&key) => {
                reserved.insert(key);
                return Ok(candidate);
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                return Err(format!("无法检查目标路径 {}: {error}", candidate.display()));
            }
            _ => {}
        }

        suffix += 1;
        let (stem, extension) = match desired_name.rfind('.') {
            Some(index) if index > 0 => (&desired_name[..index], &desired_name[index..]),
            _ => (desired_name, ""),
        };
        name = format!("{stem} ({suffix}){extension}");
    }
}

pub fn is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 复制时不合并目录、不覆盖文件。源路径经 `canonicalize` 后可能带有 `\\?\` 前缀，
/// 尚未创建的目标路径无法用同样方式规范化，因此需要单独处理 Windows 路径。
pub fn copy_path(source: &Path, target: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    let resolve = |path: &Path| {
        settings::resolve_path(path)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
    };
    let source = resolve(source)?;
    let target = resolve(target)?;
    if settings::paths_equal(&source, &target)
        || (metadata.is_dir() && settings::path_is_within(&target, &source))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("目标不能位于源目录内部: {}", target.display()),
        ));
    }
    if is_reparse_or_symlink(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("不支持复制符号链接或目录重解析点: {}", source.display()),
        ));
    }

    if metadata.is_dir() {
        fs::create_dir(&target)?;
        for entry in fs::read_dir(&source)? {
            let entry = entry?;
            copy_path(&entry.path(), &target.join(entry.file_name()))?;
        }
        fs::set_permissions(&target, metadata.permissions())
    } else {
        let mut input = fs::File::open(&source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        io::copy(&mut input, &mut output)?;
        fs::set_permissions(&target, metadata.permissions())
    }
}

pub fn ensure_distinct_target(source: &Path, target: &Path) -> Result<(), String> {
    let source = settings::resolve_path(source)?;
    let target = settings::resolve_path(target)?;
    if settings::paths_equal(&source, &target) {
        return Err(format!("源和目标不能相同: {}", source.display()));
    }
    if fs::symlink_metadata(&source)
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
        && settings::path_is_within(&target, &source)
    {
        return Err(format!(
            "不能把文件夹复制或移动到它自己的子目录: {}",
            target.display()
        ));
    }
    Ok(())
}

pub fn remove_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !is_reparse_or_symlink(&metadata) => {
            fs::remove_dir_all(path).map_err(|error| error.to_string())
        }
        Ok(_) => fs::remove_file(path).map_err(|error| error.to_string()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub struct ExportCopyOutcome {
    pub warning: Option<String>,
}

/// 先在同目录完成临时副本，再发布到目标名。覆盖时保留旧目标，发布失败可原位恢复。
pub fn copy_for_export(
    source: &Path,
    target: &Path,
    destination: &Path,
    overwrite: bool,
    reserved: &mut HashSet<String>,
) -> Result<ExportCopyOutcome, String> {
    let temporary_name = format!(".floepod-export-{}-{}", std::process::id(), db::now_ms());
    let temporary = unique_target(destination, &temporary_name, reserved)?;
    if let Err(error) = copy_path(source, &temporary) {
        let cleanup = remove_path(&temporary).err();
        return Err(match cleanup {
            Some(cleanup) => format!("复制临时副本失败: {error}；清理失败: {cleanup}"),
            None => format!("复制临时副本失败: {error}"),
        });
    }

    let backup = match fs::symlink_metadata(target) {
        Ok(metadata) => {
            if is_reparse_or_symlink(&metadata) {
                let _ = remove_path(&temporary);
                return Err("目标名称指向符号链接或目录重解析点".into());
            }
            if !overwrite {
                let _ = remove_path(&temporary);
                return Err("目标在导出过程中已出现，请重新选择冲突策略".into());
            }
            let backup_name = format!(
                ".floepod-overwrite-backup-{}-{}",
                std::process::id(),
                db::now_ms()
            );
            let backup = match unique_target(destination, &backup_name, reserved) {
                Ok(backup) => backup,
                Err(error) => {
                    let cleanup = remove_path(&temporary).err();
                    return Err(match cleanup {
                        Some(cleanup) => {
                            format!("无法为旧目标分配备份名: {error}；清理临时副本失败: {cleanup}")
                        }
                        None => format!("无法为旧目标分配备份名: {error}"),
                    });
                }
            };
            if let Err(error) = fs::rename(target, &backup) {
                let cleanup = remove_path(&temporary).err();
                return Err(match cleanup {
                    Some(cleanup) => {
                        format!("旧目标无法暂存为同目录备份: {error}；清理临时副本失败: {cleanup}")
                    }
                    None => format!("旧目标无法暂存为同目录备份: {error}"),
                });
            }
            Some(backup)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            let cleanup = remove_path(&temporary).err();
            return Err(match cleanup {
                Some(cleanup) => format!("无法检查目标路径: {error}；清理临时副本失败: {cleanup}"),
                None => format!("无法检查目标路径: {error}"),
            });
        }
    };

    if let Err(error) = fs::rename(&temporary, target) {
        let cleanup = remove_path(&temporary).err();
        let restore = backup
            .as_ref()
            .and_then(|backup| match fs::symlink_metadata(target) {
                Err(check) if check.kind() == io::ErrorKind::NotFound => {
                    fs::rename(backup, target).err().map(|restore| {
                        format!("恢复旧目标失败: {restore}；备份保留于 {}", backup.display())
                    })
                }
                Ok(_) => Some(format!(
                    "目标名称被其他程序占用；旧目标备份保留于 {}",
                    backup.display()
                )),
                Err(check) => Some(format!(
                    "无法检查目标以恢复旧文件: {check}；备份保留于 {}",
                    backup.display()
                )),
            });
        let mut details = vec![format!("最终写入失败: {error}")];
        if let Some(cleanup) = cleanup {
            details.push(format!("清理临时副本失败: {cleanup}"));
        }
        if let Some(restore) = restore {
            details.push(restore);
        }
        return Err(details.join("；"));
    }

    let warning = backup.and_then(|backup| {
        trash::delete(&backup).err().map(|error| {
            format!(
                "新目标已写入，但旧目标备份无法移入回收站: {error}；备份保留于 {}",
                backup.display()
            )
        })
    });
    Ok(ExportCopyOutcome { warning })
}

#[derive(Debug)]
pub struct StagedMove {
    pub staged: PathBuf,
    pub original: PathBuf,
    /// 跨盘移动在 SQLite 提交前将源文件保留为同目录临时名，使回滚只需同盘重命名。
    pub quarantine: Option<PathBuf>,
}

fn internal_path(parent: &Path, label: &str) -> Result<PathBuf, String> {
    for _ in 0..1024 {
        let sequence = OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".floepod-{label}-{}-{sequence:016x}",
            std::process::id()
        ));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Err(error) => {
                return Err(format!(
                    "无法检查内部临时路径 {}: {error}",
                    candidate.display()
                ));
            }
            Ok(_) => {}
        }
    }
    Err("无法分配内部临时路径".into())
}

fn restore_quarantined_move(record: &StagedMove) -> Result<(), String> {
    let quarantine = record
        .quarantine
        .as_ref()
        .ok_or_else(|| "缺少跨盘移动恢复路径".to_string())?;
    match fs::symlink_metadata(&record.original) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::rename(quarantine, &record.original).map_err(|error| {
                format!("恢复源路径 {} 失败: {error}", record.original.display())
            })?;
        }
        Ok(_) => return Err("原路径已被占用，未覆盖恢复".into()),
        Err(error) => return Err(format!("无法检查原路径: {error}")),
    }
    remove_path(&record.staged).map_err(|error| format!("源已恢复，但暂存副本清理失败: {error}"))
}

fn restore_moved_path(staged: &Path, original: &Path) -> Result<(), String> {
    match fs::symlink_metadata(original) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::rename(staged, original)
            .map_err(|error| error.to_string())
            .or_else(|_| {
                copy_path(staged, original).map_err(|error| error.to_string())?;
                remove_path(staged)
            }),
        Ok(_) => Err("原路径已被占用，未自动覆盖".into()),
        Err(error) => Err(format!("无法检查原路径: {error}")),
    }
}

pub fn rollback_staged_moves(records: &[StagedMove]) -> Vec<String> {
    records
        .iter()
        .rev()
        .filter_map(|record| {
            let result = if record.quarantine.is_some() {
                restore_quarantined_move(record)
            } else {
                restore_moved_path(&record.staged, &record.original)
            };
            result.err().map(|error| {
                format!(
                    "{} -> {}: {error}",
                    record.staged.display(),
                    record.original.display()
                )
            })
        })
        .collect()
}

/// 发布移动结果时，避免留下已经复制完成但尚未入库的跨盘文件。
pub fn move_into_staging(source: &Path, target: &Path) -> Result<StagedMove, String> {
    match fs::rename(source, target) {
        Ok(()) => Ok(StagedMove {
            staged: target.to_path_buf(),
            original: source.to_path_buf(),
            quarantine: None,
        }),
        Err(direct_error) => {
            let source_parent = source
                .parent()
                .ok_or_else(|| format!("源路径没有父目录: {}", source.display()))?;
            let quarantine = internal_path(source_parent, "move-source")?;
            let target_parent = target
                .parent()
                .ok_or_else(|| format!("目标路径没有父目录: {}", target.display()))?;
            let temporary = internal_path(target_parent, "inflight")?;
            fs::rename(source, &quarantine).map_err(|error| {
                format!("无法锁定跨盘移动源（直接移动错误: {direct_error}）：{error}")
            })?;

            let publish = (|| -> Result<(), String> {
                copy_path(&quarantine, &temporary)
                    .map_err(|error| format!("复制跨盘移动源失败: {error}"))?;
                fs::rename(&temporary, target)
                    .map_err(|error| format!("发布跨盘移动副本失败: {error}"))
            })();
            if let Err(error) = publish {
                let mut rollback_errors = Vec::new();
                if let Err(cleanup) = remove_path(&temporary) {
                    rollback_errors.push(format!("清理临时副本失败: {cleanup}"));
                }
                if let Err(restore) = fs::rename(&quarantine, source) {
                    rollback_errors.push(format!("恢复源路径失败: {restore}"));
                }
                return Err(if rollback_errors.is_empty() {
                    error
                } else {
                    format!("{error}；{}", rollback_errors.join("；"))
                });
            }

            Ok(StagedMove {
                staged: target.to_path_buf(),
                original: source.to_path_buf(),
                quarantine: Some(quarantine),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_target_reserves_names_with_extensions() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(temporary.path().join("a.pdf"), b"x").unwrap();
        let mut reserved = HashSet::new();
        assert_eq!(
            unique_target(temporary.path(), "a.pdf", &mut reserved)
                .unwrap()
                .file_name()
                .unwrap(),
            "a (2).pdf"
        );
        assert_eq!(
            unique_target(temporary.path(), "a.pdf", &mut reserved)
                .unwrap()
                .file_name()
                .unwrap(),
            "a (3).pdf"
        );
    }

    #[test]
    fn copy_path_never_overwrites_or_merges() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("root.txt"), b"root").unwrap();
        fs::write(source.join("nested/child.bin"), b"child").unwrap();
        let target = temporary.path().join("target");

        copy_path(&source, &target).unwrap();
        assert_eq!(fs::read(target.join("root.txt")).unwrap(), b"root");
        assert_eq!(fs::read(target.join("nested/child.bin")).unwrap(), b"child");
        fs::write(target.join("keep.txt"), b"keep").unwrap();
        assert!(copy_path(&source, &target).is_err());
        assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"keep");
    }

    #[test]
    fn rejects_equal_and_descendant_targets() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        fs::create_dir(&source).unwrap();
        assert!(ensure_distinct_target(&source, &source).is_err());
        assert!(ensure_distinct_target(&source, &source.join("nested")).is_err());
        assert!(copy_path(&source, &source.join("nested")).is_err());
    }

    #[test]
    fn extension_preserves_legacy_edge_cases() {
        assert_eq!(extension("a.PDF").as_deref(), Some("pdf"));
        assert_eq!(extension(".gitignore"), None);
        assert_eq!(extension("noext"), None);
        assert_eq!(extension("arch.tar.gz").as_deref(), Some("gz"));
    }
}
