use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::db;

static OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Atomic same-volume publication without replacing a name created by another process.
pub fn rename_new(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let wide = |path: &Path| -> io::Result<Vec<u16>> {
        let path = crate::file_paths::resolve_path(path)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "路径包含空字符",
            ));
        }
        value.push(0);
        Ok(value)
    };
    let source = wide(source)?;
    let target = wide(target)?;
    // No REPLACE_EXISTING and no COPY_ALLOWED: cross-volume copies belong to our journaled protocol.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

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
    let mut components = Path::new(desired_name).components();
    if desired_name.is_empty()
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err("目标名称必须是单个文件名，不能包含路径".into());
    }
    let mut name = desired_name.to_string();
    let mut suffix = 1;
    loop {
        let candidate = directory.join(&name);
        let key = crate::file_paths::path_key(&crate::file_paths::resolve_path(&candidate)?);
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

/// watcher 对账必须跳过的应用内部临时名（半成品副本 / 覆盖备份 / 跨盘移动源）。
/// 清理失败的残留一旦被索引，就会永久变成"幽灵条目"，因此前缀必须集中在这里。
pub fn is_internal_temp_name(name: &str) -> bool {
    name.starts_with(".floepod-inflight-")
        || name.starts_with(".floepod-move-source-")
        || name.starts_with(".floepod-export-")
        || name.starts_with(".floepod-privacy-")
        || name.starts_with(".floepod-overwrite-backup-")
}

/// 目录复制的深度上限：文件系统路径长度本身已经限制了合理深度，
/// 这里只是防御病态构造的目录树耗尽线程栈。
const MAX_COPY_DEPTH: usize = 512;

/// 复制时不合并目录、不覆盖文件。源路径经 `canonicalize` 后可能带有 `\\?\` 前缀，
/// 尚未创建的目标路径无法用同样方式规范化，因此需要单独处理 Windows 路径。
pub fn copy_path(source: &Path, target: &Path) -> io::Result<()> {
    copy_path_inner(source, target, 0)
}

fn copy_path_inner(source: &Path, target: &Path, depth: usize) -> io::Result<()> {
    if depth > MAX_COPY_DEPTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "目录嵌套超过 {MAX_COPY_DEPTH} 层，拒绝复制: {}",
                source.display()
            ),
        ));
    }
    let metadata = fs::symlink_metadata(source)?;
    let resolve = |path: &Path| {
        crate::file_paths::resolve_path(path)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
    };
    let source = resolve(source)?;
    let target = resolve(target)?;
    if crate::file_paths::paths_equal(&source, &target)
        || (metadata.is_dir() && crate::file_paths::path_is_within(&target, &source))
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
        let result = (|| {
            for entry in fs::read_dir(&source)? {
                let entry = entry?;
                copy_path_inner(&entry.path(), &target.join(entry.file_name()), depth + 1)?;
            }
            fs::set_permissions(&target, metadata.permissions())
        })();
        if let Err(error) = &result {
            // Only this successfully created directory belongs to this attempt.
            // Preserve cleanup errors so callers do not delete an unowned target.
            if let Err(cleanup) = remove_path(&target) {
                return Err(io::Error::other(format!(
                    "{}；清理本次副本失败: {cleanup}",
                    error
                )));
            }
        }
        result
    } else {
        let mut input = fs::File::open(&source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        let result = io::copy(&mut input, &mut output)
            .and_then(|_| output.sync_all())
            .and_then(|_| fs::set_permissions(&target, metadata.permissions()));
        drop(output);
        if let Err(error) = &result {
            if let Err(cleanup) = remove_path(&target) {
                return Err(io::Error::other(format!(
                    "{}；清理本次副本失败: {cleanup}",
                    error
                )));
            }
        }
        result
    }
}

pub fn ensure_distinct_target(source: &Path, target: &Path) -> Result<(), String> {
    let source = crate::file_paths::resolve_path(source)?;
    let target = crate::file_paths::resolve_path(target)?;
    if crate::file_paths::paths_equal(&source, &target) {
        return Err(format!("源和目标不能相同: {}", source.display()));
    }
    if fs::symlink_metadata(&source)
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
        && crate::file_paths::path_is_within(&target, &source)
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
        return Err(format!("复制临时副本失败: {error}"));
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
            if let Err(error) = rename_new(target, &backup) {
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

    if let Err(error) = rename_new(&temporary, target) {
        let cleanup = remove_path(&temporary).err();
        let restore = backup
            .as_ref()
            .and_then(|backup| match fs::symlink_metadata(target) {
                Err(check) if check.kind() == io::ErrorKind::NotFound => {
                    rename_new(backup, target).err().map(|restore| {
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
            rename_new(quarantine, &record.original).map_err(|error| {
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
        Err(error) if error.kind() == io::ErrorKind::NotFound => rename_new(staged, original)
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
///
/// Persist the recovery record before hiding the source under its quarantine name.
pub fn move_into_staging(
    source: &Path,
    target: &Path,
    journal: impl FnOnce(&StagedMove) -> Result<(), String>,
) -> Result<StagedMove, String> {
    move_into_staging_using(source, target, journal, rename_new)
}

fn move_into_staging_using(
    source: &Path,
    target: &Path,
    journal: impl FnOnce(&StagedMove) -> Result<(), String>,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Result<StagedMove, String> {
    match rename(source, target) {
        Ok(()) => Ok(StagedMove {
            staged: target.to_path_buf(),
            original: source.to_path_buf(),
            quarantine: None,
        }),
        Err(direct_error) if direct_error.raw_os_error() == Some(17) => {
            let source_parent = source
                .parent()
                .ok_or_else(|| format!("源路径没有父目录: {}", source.display()))?;
            let quarantine = internal_path(source_parent, "move-source")?;
            let target_parent = target
                .parent()
                .ok_or_else(|| format!("目标路径没有父目录: {}", target.display()))?;
            let temporary = internal_path(target_parent, "inflight")?;
            let record = StagedMove {
                staged: target.to_path_buf(),
                original: source.to_path_buf(),
                quarantine: Some(quarantine.clone()),
            };
            journal(&record)?;
            rename_new(source, &quarantine).map_err(|error| {
                format!("无法锁定跨盘移动源（直接移动错误: {direct_error}）：{error}")
            })?;

            let mut copied = false;
            let publish = (|| -> Result<(), String> {
                copy_path(&quarantine, &temporary)
                    .map_err(|error| format!("复制跨盘移动源失败: {error}"))?;
                copied = true;
                rename_new(&temporary, target)
                    .map_err(|error| format!("发布跨盘移动副本失败: {error}"))
            })();
            if let Err(error) = publish {
                let mut rollback_errors = Vec::new();
                if copied {
                    if let Err(cleanup) = remove_path(&temporary) {
                        rollback_errors.push(format!("清理临时副本失败: {cleanup}"));
                    }
                }
                if let Err(restore) = rename_new(&quarantine, source) {
                    rollback_errors.push(format!("恢复源路径失败: {restore}"));
                }
                return Err(if rollback_errors.is_empty() {
                    error
                } else {
                    format!("{error}；{}", rollback_errors.join("；"))
                });
            }

            Ok(record)
        }
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_claimed_after_planning_is_never_replaced_or_removed() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.txt");
        fs::write(&source, b"source").unwrap();
        let target = unique_target(directory.path(), "target.txt", &mut HashSet::new()).unwrap();
        fs::write(&target, b"another application").unwrap();
        assert!(rename_new(&source, &target).is_err());
        assert!(copy_path(&source, &target).is_err());
        assert!(move_into_staging(&source, &target, |_| panic!("not cross volume")).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(fs::read(&target).unwrap(), b"another application");
    }

    #[test]
    fn failed_copy_removes_only_the_directory_it_created() {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        fs::create_dir(&source).unwrap();
        let locked_path = source.join("locked.txt");
        fs::write(&locked_path, b"cannot read now").unwrap();
        let _locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked_path)
            .unwrap();
        let target = directory.path().join("target");
        assert!(copy_path(&source, &target).is_err());
        assert!(!target.exists());
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep.txt"), b"keep").unwrap();
        assert!(copy_path(&source, &target).is_err());
        assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"keep");
    }

    #[test]
    fn cross_volume_journal_failure_leaves_the_source_at_its_original_name() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.txt");
        let target = directory.path().join("target.txt");
        fs::write(&source, b"original").unwrap();
        let error = move_into_staging_using(
            &source,
            &target,
            |record| {
                assert!(source.exists());
                assert!(!record.quarantine.as_ref().unwrap().exists());
                Err("database is full".into())
            },
            |_, _| Err(io::Error::from_raw_os_error(17)),
        )
        .unwrap_err();
        assert_eq!(error, "database is full");
        assert_eq!(fs::read(&source).unwrap(), b"original");
        assert!(!target.exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn cross_volume_publish_collision_restores_the_source_without_overwriting_the_target() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.txt");
        let target = directory.path().join("target.txt");
        fs::write(&source, b"original").unwrap();
        let result = move_into_staging_using(
            &source,
            &target,
            |_| {
                fs::write(&target, b"another application").unwrap();
                Ok(())
            },
            |_, _| Err(io::Error::from_raw_os_error(17)),
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&source).unwrap(), b"original");
        assert_eq!(fs::read(&target).unwrap(), b"another application");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn cross_volume_move_retains_the_source_until_commit_and_rolls_back() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.txt");
        let target = directory.path().join("target.txt");
        fs::write(&source, b"original").unwrap();
        let record = move_into_staging_using(
            &source,
            &target,
            |_| Ok(()),
            |_, _| Err(io::Error::from_raw_os_error(17)),
        )
        .unwrap();
        assert!(!source.exists());
        assert_eq!(
            fs::read(record.quarantine.as_ref().unwrap()).unwrap(),
            b"original"
        );
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert!(rollback_staged_moves(&[record]).is_empty());
        assert_eq!(fs::read(&source).unwrap(), b"original");
        assert!(!target.exists());
    }

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
    fn unique_target_rejects_path_components() {
        let temporary = tempfile::tempdir().unwrap();
        let mut reserved = HashSet::new();
        assert!(unique_target(temporary.path(), "../outside.txt", &mut reserved).is_err());
        assert!(unique_target(temporary.path(), r"folder\\file.txt", &mut reserved).is_err());
        assert!(unique_target(temporary.path(), "", &mut reserved).is_err());
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

    #[test]
    fn internal_temp_names_cover_every_internal_prefix() {
        for name in [
            ".floepod-inflight-1-0000000000000001",
            ".floepod-move-source-1-0000000000000002",
            ".floepod-export-1-1",
            ".floepod-privacy-1-1",
            ".floepod-overwrite-backup-1-1",
        ] {
            assert!(is_internal_temp_name(name), "{name}");
        }
        assert!(!is_internal_temp_name(".floepod-unrelated"));
        assert!(!is_internal_temp_name("notes.txt"));
    }
}
