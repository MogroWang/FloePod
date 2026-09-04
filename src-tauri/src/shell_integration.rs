//! Windows 资源管理器“发送到 FloePod”：HKCU 注册与 `--stage <path>` 分发。

use std::path::PathBuf;

use tauri::AppHandle;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const ROOTS: [&str; 2] = [
    r"Software\Classes\*\shell\FloePod",
    r"Software\Classes\Directory\shell\FloePod",
];

fn command_line() -> Result<(String, String), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let executable = executable.to_string_lossy().to_string();
    Ok((
        executable.clone(),
        format!("\"{executable}\" --stage \"%1\""),
    ))
}

pub fn sync(enabled: bool) -> Result<(), String> {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    if !enabled {
        for root in ROOTS {
            if let Err(error) = current_user.delete_subkey_all(root) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(format!("移除资源管理器菜单失败: {error}"));
                }
            }
        }
        return Ok(());
    }
    let (icon, command) = command_line()?;
    for root in ROOTS {
        let (menu, _) = current_user
            .create_subkey(root)
            .map_err(|error| format!("创建资源管理器菜单失败: {error}"))?;
        menu.set_value("", &"发送到 FloePod")
            .map_err(|error| error.to_string())?;
        menu.set_value("Icon", &icon)
            .map_err(|error| error.to_string())?;
        menu.set_value("MultiSelectModel", &"Player")
            .map_err(|error| error.to_string())?;
        let (command_key, _) = menu
            .create_subkey("command")
            .map_err(|error| error.to_string())?;
        command_key
            .set_value("", &command)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn stage_paths_from_args(args: &[String]) -> Vec<String> {
    let Some(position) = args.iter().position(|arg| arg == "--stage") else {
        return Vec::new();
    };
    args[position + 1..]
        .iter()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.exists())
        .map(|path| path.to_string_lossy().to_string())
        .collect()
}

/// 返回是否识别到有效的 --stage 文件参数。
pub fn handle_args(app: &AppHandle, args: Vec<String>) -> bool {
    let paths = stage_paths_from_args(&args);
    if paths.is_empty() {
        return false;
    }
    let settings = crate::manager::current_settings(app);
    let pod_id = settings
        .pods
        .into_iter()
        .find(|pod| pod.enabled && !crate::security::is_locked(app, pod.id))
        .map(|pod| pod.id);
    let Some(pod_id) = pod_id else {
        crate::logging::write("[shell] 没有可接收资源管理器投递的已解锁匣");
        return true;
    };
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = crate::staging::stage_paths(app, pod_id, paths, "copy".into()) {
            crate::logging::write(&format!("[shell] 资源管理器投递失败: {error}"));
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quotes_executable_and_selected_path() {
        let (_, command) = command_line().unwrap();
        assert!(command.starts_with('"'));
        assert!(command.ends_with("--stage \"%1\""));
    }

    #[test]
    fn unrelated_arguments_do_not_stage_files() {
        assert!(stage_paths_from_args(&["FloePod.exe".into(), "--other".into()]).is_empty());
    }
}
