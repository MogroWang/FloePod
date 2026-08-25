use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const PORTABLE_MARKER: &str = ".floepod-portable";

pub fn resolve() -> PathBuf {
    static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
    DATA_DIR.get_or_init(resolve_uncached).clone()
}

fn resolve_uncached() -> PathBuf {
    resolve_from(
        std::env::current_exe()
            .ok()
            .and_then(|executable| executable.parent().map(Path::to_path_buf)),
        std::env::var_os("APPDATA").map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        std::env::temp_dir(),
    )
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
        // Even in a restricted environment, never degrade to a relative
        // `FloePod` directory beside the current working directory.
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
