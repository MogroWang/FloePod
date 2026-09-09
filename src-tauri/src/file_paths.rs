//! 文件操作与配置共用的路径身份和包含关系；配置离线容忍不延伸到实际 I/O。
use std::path::{Component, Path, PathBuf};

/// 将绝对路径做词法归一化，并尽可能解析已存在祖先中的符号链接 / junction。
///
/// 暂存目录允许尚不存在，因此不能简单要求 `canonicalize()` 整条路径成功。
pub fn resolve_path(path: &Path) -> Result<PathBuf, String> {
    resolve_path_impl(path, false)
}

/// 配置校验允许可移动盘暂时离线；这种情况下只能保留词法归一化结果。
/// 真正读写文件时仍必须使用 [`resolve_path`]，从而把“盘符不可用”与“叶子不存在”区分开。
pub(crate) fn resolve_config_path(path: &Path) -> Result<PathBuf, String> {
    resolve_path_impl(path, true)
}

/// 比较两条持久化路径时不要求磁盘或共享在线；实际文件操作仍须先调用 [`resolve_path`]。
pub fn configured_paths_equal(a: &Path, b: &Path) -> Result<bool, String> {
    Ok(paths_equal(
        &resolve_config_path(a)?,
        &resolve_config_path(b)?,
    ))
}

fn resolve_path_impl(path: &Path, allow_missing_root: bool) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("路径必须是绝对路径: {}", path.display()));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(format!("路径越过根目录: {}", path.display()));
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    let mut cursor = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(cursor) {
            Ok(_) => {
                let mut resolved = cursor
                    .canonicalize()
                    .map_err(|e| format!("无法解析路径 {}: {e}", cursor.display()))?;
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = cursor.file_name() else {
                    return if allow_missing_root {
                        Ok(normalized)
                    } else {
                        Err(format!("路径所在磁盘或共享位置不可用: {}", path.display()))
                    };
                };
                missing.push(name.to_os_string());
                cursor = cursor
                    .parent()
                    .ok_or_else(|| format!("无法解析路径: {}", path.display()))?;
            }
            Err(e) => return Err(format!("无法访问路径 {}: {e}", cursor.display())),
        }
    }
}

fn component_eq(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// `path` 是否等于 `root` 或位于其下。调用方应先用 [`resolve_path`] 归一化。
pub fn path_is_within(path: &Path, root: &Path) -> bool {
    let path_parts: Vec<_> = path.components().map(|c| c.as_os_str()).collect();
    let root_parts: Vec<_> = root.components().map(|c| c.as_os_str()).collect();
    root_parts.len() <= path_parts.len()
        && root_parts
            .iter()
            .zip(path_parts.iter())
            .all(|(a, b)| component_eq(a, b))
}

pub fn paths_equal(a: &Path, b: &Path) -> bool {
    let a_parts: Vec<_> = a.components().map(|c| c.as_os_str()).collect();
    let b_parts: Vec<_> = b.components().map(|c| c.as_os_str()).collect();
    a_parts.len() == b_parts.len()
        && a_parts
            .iter()
            .zip(b_parts.iter())
            .all(|(x, y)| component_eq(x, y))
}

pub fn path_key(path: &Path) -> String {
    #[cfg(windows)]
    {
        path.to_string_lossy().replace('/', "\\").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().to_string()
    }
}
