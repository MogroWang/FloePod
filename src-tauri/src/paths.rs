use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rusqlite::Connection;

const PORTABLE_MARKER: &str = ".floepod-portable";
#[cfg(windows)]
const OVERRIDE_SUBKEY: &str = r"Software\FloePod";
#[cfg(windows)]
const OVERRIDE_VALUE: &str = "DataDir";

pub fn resolve() -> PathBuf {
    static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
    DATA_DIR.get_or_init(resolve_uncached).clone()
}

fn resolve_uncached() -> PathBuf {
    // 关于页「更改数据位置」写入的注册表覆盖项优先于便携模式与默认位置。
    if let Some(overridden) = override_data_dir() {
        return overridden;
    }
    resolve_from(
        std::env::current_exe()
            .ok()
            .and_then(|executable| executable.parent().map(Path::to_path_buf)),
        std::env::var_os("APPDATA").map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        std::env::temp_dir(),
    )
}

/// 注册表覆盖的数据目录。路径无效或不可写时忽略并回退默认解析，
/// 保证应用总能启动；损坏的覆盖项不会把应用锁死。
fn override_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let current_user = RegKey::predef(HKEY_CURRENT_USER);
        let key = current_user.open_subkey(OVERRIDE_SUBKEY).ok()?;
        let raw: String = key.get_value(OVERRIDE_VALUE).ok()?;
        let path = PathBuf::from(raw);
        if path.is_absolute() && ensure_writable(&path) {
            Some(path)
        } else {
            None
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// 写入数据目录覆盖项；重启后 `resolve()` 优先使用该位置。
pub fn set_override_data_dir(new_path: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let current_user = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = current_user
            .create_subkey(OVERRIDE_SUBKEY)
            .map_err(|error| format!("无法写入数据位置配置: {error}"))?;
        key.set_value(OVERRIDE_VALUE, &new_path)
            .map_err(|error| format!("无法写入数据位置配置: {error}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = new_path;
        Err("当前平台不支持更改数据位置".into())
    }
}

/// 递归统计数据目录占用（跳过符号链接与不可读项），单位字节。
pub fn directory_size(root: &Path) -> u64 {
    let mut total = 0u64;
    walk_size(root, &mut total);
    total
}

fn walk_size(dir: &Path, total: &mut u64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            walk_size(&path, total);
        } else if let Ok(metadata) = entry.metadata() {
            *total += metadata.len();
        }
    }
}

/// 把数据目录迁移到新位置：数据库用 VACUUM INTO 生成一致快照，撤销区
/// 递归复制（索引中的撤销记录仍指向 undo/ 下的文件）。注册表覆盖项由
/// 调用方在迁移成功后写入，本函数不触碰任何现有数据。
pub fn migrate_data_dir(
    current: &Path,
    connection: &Connection,
    new_path: &str,
) -> Result<(), String> {
    let target = PathBuf::from(new_path.trim());
    if !target.is_absolute() {
        return Err("数据位置必须是绝对路径".into());
    }
    let canonical_current = current
        .canonicalize()
        .unwrap_or_else(|_| current.to_path_buf());
    let canonical_target = if target.exists() {
        target.canonicalize().unwrap_or_else(|_| target.clone())
    } else {
        target.clone()
    };
    if canonical_current == canonical_target {
        return Err("新数据位置与当前位置相同".into());
    }
    if crate::file_paths::path_is_within(&canonical_target, &canonical_current)
        || crate::file_paths::path_is_within(&canonical_current, &canonical_target)
    {
        return Err("新数据位置不能与当前数据目录互相包含".into());
    }
    fs::create_dir_all(&target)
        .map_err(|error| format!("无法创建目录 {}: {error}", target.display()))?;
    if !ensure_writable(&target) {
        return Err(format!("目录不可写: {}", target.display()));
    }
    let target_db = target.join("data.db");
    if target_db.exists() {
        return Err(format!("目标目录已包含 FloePod 数据: {}", target.display()));
    }
    connection
        .execute("VACUUM INTO ?1", [target_db.to_string_lossy().as_ref()])
        .map_err(|error| format!("数据库快照失败: {error}"))?;
    let undo_source = current.join("undo");
    let undo_target = target.join("undo");
    if undo_source.is_dir() {
        if let Err(error) = copy_dir_recursive(&undo_source, &undo_target) {
            // 尽力清掉半成品快照，失败原因照常返回；现有数据不受影响。
            let _ = fs::remove_file(&target_db);
            let _ = fs::remove_dir_all(&undo_target);
            return Err(format!("撤销区复制失败: {error}"));
        }
    }
    Ok(())
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source)
        .map_err(|error| error.to_string())?
        .flatten()
    {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let dest = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), &dest).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn resolve_from(
    executable_directory: Option<PathBuf>,
    roaming_app_data: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    temporary_directory: PathBuf,
) -> PathBuf {
    if let Some(directory) = executable_directory {
        let portable = directory.join("FloePodData");
        if portable_requested(&directory) && ensure_writable(&portable) {
            return portable;
        }
    }

    let base = roaming_app_data
        .filter(|path| path.is_absolute())
        .or_else(|| local_app_data.filter(|path| path.is_absolute()))
        // 受限环境中也不能退回当前工作目录旁的相对 `FloePod` 目录。
        .unwrap_or(temporary_directory);
    let installed = base.join("FloePod");
    let _ = fs::create_dir_all(&installed);
    installed
}

fn portable_requested(executable_directory: &Path) -> bool {
    executable_directory.join(PORTABLE_MARKER).is_file()
        || executable_directory.join("FloePodData").is_dir()
}

fn ensure_writable(directory: &Path) -> bool {
    if fs::create_dir_all(directory).is_err() {
        return false;
    }
    let probe = directory.join(format!(".write-probe-{}", std::process::id()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_marker_and_existing_data_preserve_upgrade_paths() {
        let temporary = tempfile::tempdir().unwrap();
        let executable = temporary.path().join("portable");
        let roaming = temporary.path().join("roaming");
        fs::create_dir_all(&executable).unwrap();

        fs::write(executable.join(PORTABLE_MARKER), b"portable").unwrap();
        assert_eq!(
            resolve_from(
                Some(executable.clone()),
                Some(roaming.clone()),
                None,
                temporary.path().join("temp"),
            ),
            executable.join("FloePodData")
        );

        fs::remove_file(executable.join(PORTABLE_MARKER)).unwrap();
        assert!(portable_requested(&executable));
        assert_eq!(
            resolve_from(
                Some(executable.clone()),
                Some(roaming),
                None,
                temporary.path().join("temp"),
            ),
            executable.join("FloePodData")
        );
    }

    #[test]
    fn writable_program_directory_without_marker_stays_installed() {
        let temporary = tempfile::tempdir().unwrap();
        let executable = temporary.path().join("installed-program");
        let roaming = temporary.path().join("roaming");
        fs::create_dir_all(&executable).unwrap();

        assert_eq!(
            resolve_from(
                Some(executable),
                Some(roaming.clone()),
                None,
                temporary.path().join("temp"),
            ),
            roaming.join("FloePod")
        );
    }

    #[test]
    fn installed_fallbacks_require_absolute_roots() {
        let temporary = tempfile::tempdir().unwrap();
        let local = temporary.path().join("local");
        assert_eq!(
            resolve_from(
                None,
                Some(PathBuf::from("relative-roaming")),
                Some(local.clone()),
                temporary.path().join("temp"),
            ),
            local.join("FloePod")
        );

        let fallback = temporary.path().join("temp");
        assert_eq!(
            resolve_from(
                None,
                Some(PathBuf::from("relative-roaming")),
                Some(PathBuf::from("relative-local")),
                fallback.clone(),
            ),
            fallback.join("FloePod")
        );
    }

    #[test]
    fn writable_probe_leaves_no_file() {
        let temporary = tempfile::tempdir().unwrap();
        assert!(ensure_writable(temporary.path()));
        assert!(!temporary
            .path()
            .join(format!(".write-probe-{}", std::process::id()))
            .exists());
    }
}
