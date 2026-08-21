use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const PORTABLE_MARKER: &str = ".floepod-portable";

/// 数据目录解析：便携优先。
/// 便携包通过 exe 旁的 marker 声明便携模式；已有 `FloePodData` 也继续兼容。
/// 安装版不会再仅因安装目录偶然可写而把用户数据放到程序目录。
/// 不依赖 AppHandle，可在 Builder 阶段（窗口创建前）完成状态注册。
pub fn resolve() -> PathBuf {
    static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
    DATA_DIR.get_or_init(resolve_uncached).clone()
}

fn resolve_uncached() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let portable = dir.join("FloePodData");
            if portable_requested(dir) && ensure_writable(&portable) {
                return portable;
            }
        }
    }

    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        })
        // 极端受限环境下也必须保持绝对路径，不能退化成当前目录下的 `FloePod`。
        .unwrap_or_else(std::env::temp_dir);
    let fallback = base.join("FloePod");
    let _ = fs::create_dir_all(&fallback);
    fallback
}

fn portable_requested(exe_dir: &Path) -> bool {
    exe_dir.join(PORTABLE_MARKER).is_file() || exe_dir.join("FloePodData").is_dir()
}

fn ensure_writable(dir: &Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".write-probe-{}", std::process::id()));
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
    fn writable_dir_passes_probe() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().to_path_buf();
        assert!(ensure_writable(&d));
        assert!(!d
            .join(format!(".write-probe-{}", std::process::id()))
            .exists());
    }

    #[test]
    fn portable_mode_requires_marker_or_existing_data() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!portable_requested(tmp.path()));

        fs::write(tmp.path().join(PORTABLE_MARKER), b"").unwrap();
        assert!(portable_requested(tmp.path()));

        fs::remove_file(tmp.path().join(PORTABLE_MARKER)).unwrap();
        fs::create_dir(tmp.path().join("FloePodData")).unwrap();
        assert!(portable_requested(tmp.path()));
    }
}
