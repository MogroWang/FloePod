//! Windows 资源管理器右键菜单：每个匣一个「发送到」项，HKCU 注册与
//! `--stage-to <匣ID> <path>` 分发。
//!
//! 经典静态 verb（`*\shell\` 注册表键）不支持动态子菜单，因此这里为每个
//! 启用的匣注册一个独立 verb（`FloePod.<匣ID>`），菜单文字即匣名；匣的
//! 增删或改名都会随 `apply_settings` 走到 `sync`，菜单随之重建。

use std::path::PathBuf;

use tauri::AppHandle;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

use crate::settings::Pod;

const SHELL_ROOTS: [&str; 2] = [
    r"Software\Classes\*\shell",
    r"Software\Classes\Directory\shell",
];
const VERB_PREFIX: &str = "FloePod";

/// 匣名直接进入菜单文字；截断到 40 字符，避免超长名称撑爆菜单。
fn menu_label(pod_name: &str) -> String {
    let name: String = pod_name.chars().take(40).collect();
    format!("发送到「{name}」")
}

fn command_line(pod_id: u64) -> Result<(String, String), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let executable = executable.to_string_lossy().to_string();
    Ok((
        executable.clone(),
        format!("\"{executable}\" --stage-to {pod_id} \"%1\""),
    ))
}

/// 让菜单与当前匣列表一致：先清除本应用注册过的全部 verb（含 1.7.x 的
/// 单一 `FloePod` 键与按匣的 `FloePod.<id>`），关闭时到此为止，开启时
/// 再按当前匣逐个重建。
pub fn sync(enabled: bool, pods: &[Pod]) -> Result<(), String> {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    for root in SHELL_ROOTS {
        let Ok(shell) = current_user.open_subkey(root) else {
            continue;
        };
        let stale: Vec<String> = shell
            .enum_keys()
            .filter_map(|result| result.ok())
            .filter(|name| name == VERB_PREFIX || name.starts_with(&format!("{VERB_PREFIX}.")))
            .collect();
        for name in stale {
            let path = format!("{root}\\{name}");
            if let Err(error) = current_user.delete_subkey_all(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(format!("移除资源管理器菜单失败: {error}"));
                }
            }
        }
    }
    if !enabled {
        return Ok(());
    }
    for pod in pods.iter().filter(|pod| pod.enabled) {
        let (icon, command) = command_line(pod.id)?;
        for root in SHELL_ROOTS {
            let (menu, _) = current_user
                .create_subkey(format!("{root}\\{VERB_PREFIX}.{}", pod.id))
                .map_err(|error| format!("创建资源管理器菜单失败: {error}"))?;
            menu.set_value("", &menu_label(&pod.name))
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
    }
    Ok(())
}

fn collect_paths(args: &[String]) -> Vec<String> {
    args.iter()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.exists())
        .map(|path| path.to_string_lossy().to_string())
        .collect()
}

/// 解析菜单注册的启动参数。返回目标匣 ID（旧格式没有）与文件路径列表。
fn stage_request_from_args(args: &[String]) -> Option<(Option<u64>, Vec<String>)> {
    if let Some(position) = args.iter().position(|arg| arg == "--stage-to") {
        let pod_id = args
            .get(position + 1)
            .and_then(|value| value.parse::<u64>().ok());
        return Some((pod_id, collect_paths(&args[position + 2..])));
    }
    let position = args.iter().position(|arg| arg == "--stage")?;
    Some((None, collect_paths(&args[position + 1..])))
}

/// 返回是否识别到有效的文件投递参数。菜单点选的匣不可用（停用 / 锁定 /
/// 已删除）时回退到第一个可用匣，避免右键菜单静默失效。
pub fn handle_args(app: &AppHandle, args: Vec<String>) -> bool {
    let Some((requested_pod, paths)) = stage_request_from_args(&args) else {
        return false;
    };
    if paths.is_empty() {
        return true;
    }
    let settings = crate::manager::current_settings(app);
    let requested = requested_pod.and_then(|id| {
        settings
            .pods
            .iter()
            .find(|pod| pod.id == id && pod.enabled && !crate::security::is_locked(app, pod.id))
            .map(|pod| pod.id)
    });
    let pod_id = requested.or_else(|| {
        settings
            .pods
            .iter()
            .find(|pod| pod.enabled && !crate::security::is_locked(app, pod.id))
            .map(|pod| pod.id)
    });
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
        let (_, command) = command_line(3).unwrap();
        assert!(command.starts_with('"'));
        assert!(command.ends_with("--stage-to 3 \"%1\""));
    }

    #[test]
    fn stage_to_parses_pod_id_and_paths() {
        let request = stage_request_from_args(&[
            "FloePod.exe".into(),
            "--stage-to".into(),
            "2".into(),
            "C:\\a.txt".into(),
        ])
        .unwrap();
        assert_eq!(request.0, Some(2));
        assert_eq!(request.1, vec!["C:\\a.txt".to_string()]);
    }

    #[test]
    fn legacy_stage_argument_has_no_pod_id() {
        let request =
            stage_request_from_args(&["FloePod.exe".into(), "--stage".into(), "C:\\b.txt".into()])
                .unwrap();
        assert_eq!(request.0, None);
        assert_eq!(request.1, vec!["C:\\b.txt".to_string()]);
    }

    #[test]
    fn unrelated_arguments_do_not_stage_files() {
        assert!(stage_request_from_args(&["FloePod.exe".into(), "--other".into()]).is_none());
    }
}
