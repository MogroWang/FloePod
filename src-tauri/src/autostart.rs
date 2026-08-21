//! Windows 开机自启动。
//!
//! Tauri autostart 插件在 Windows 上把未加引号的可执行文件路径写入同一个
//! `FloePod` 注册表值。安装路径含空格时会被 Windows 错误拆分，而且便携版与
//! 安装版会互相覆盖或删除。这里直接管理 HKCU Run：始终引用路径，并用规范化
//! 可执行文件路径的稳定散列作为实例专属值名。

const VALUE_PREFIX: &str = "FloePod-";
const LEGACY_VALUE_NAME: &str = "FloePod";

/// 同步当前可执行文件实例的 Windows HKCU Run 注册表值。
#[cfg(windows)]
pub fn sync(enabled: bool) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| format!("无法获取当前程序路径: {e}"))?;
    let executable = executable
        .to_str()
        .ok_or_else(|| "当前程序路径不是有效的 Unicode，无法安全配置自启动".to_string())?;
    let normalized = normalize_windows_path_key(executable)
        .ok_or_else(|| format!("当前程序路径不是有效的 Windows 绝对路径: {executable}"))?;
    let value_name = instance_value_name(&normalized);
    let command = quote_executable(executable)
        .ok_or_else(|| "当前程序路径包含不能写入启动命令的字符".to_string())?;

    registry::sync(&value_name, &command, &normalized, enabled)
}

#[cfg(not(windows))]
pub fn sync(_enabled: bool) -> Result<(), String> {
    Err("FloePod 的开机自启动目前仅支持 Windows".to_string())
}

/// 将 Windows 绝对路径变为用于实例身份的稳定 key。
///
/// 统一扩展路径前缀、大小写、分隔符及 `.`/`..` 组件。对于 UNC 路径，
/// `..` 不得越过 server/share；对于盘符路径，不得越过根目录。
fn normalize_windows_path_key(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.contains(['\0', '"']) {
        return None;
    }

    let replaced = raw.replace('/', "\\");
    let path = if starts_with_ignore_ascii_case(&replaced, r"\\?\UNC\") {
        format!(r"\\{}", &replaced[8..])
    } else if starts_with_ignore_ascii_case(&replaced, r"\\?\") {
        replaced[4..].to_string()
    } else {
        replaced
    };

    if let Some(rest) = path.strip_prefix(r"\\") {
        let mut components = Vec::new();
        for component in rest.split('\\') {
            match component {
                "" | "." => {}
                ".." if components.len() > 2 => {
                    components.pop();
                }
                ".." => return None,
                value => components.push(value.to_lowercase()),
            }
        }
        if components.len() < 2 {
            return None;
        }
        return Some(format!(r"\\{}", components.join(r"\")));
    }

    let bytes = path.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return None;
    }

    let drive = (bytes[0] as char).to_ascii_lowercase();
    let mut components: Vec<String> = Vec::new();
    for component in path[3..].split('\\') {
        match component {
            "" | "." => {}
            ".." if !components.is_empty() => {
                components.pop();
            }
            ".." => return None,
            value => components.push(value.to_lowercase()),
        }
    }

    if components.is_empty() {
        Some(format!(r"{drive}:\"))
    } else {
        Some(format!(r"{drive}:\{}", components.join(r"\")))
    }
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// FNV-1a 128 位散列是固定算法，不依赖 Rust 版本或进程随机种子；完整 128 位
/// 输出使不同安装/便携路径发生实例名碰撞的概率可以忽略。
fn instance_value_name(normalized_executable: &str) -> String {
    const FNV_OFFSET_BASIS: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const FNV_PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

    let hash = normalized_executable
        .as_bytes()
        .iter()
        .fold(FNV_OFFSET_BASIS, |hash, byte| {
            (hash ^ u128::from(*byte)).wrapping_mul(FNV_PRIME)
        });
    format!("{VALUE_PREFIX}{hash:032x}")
}

/// HKCU Run 的命令行始终给可执行文件加双引号。Windows 文件名不能包含双引号；
/// 遇到异常字符串时拒绝写入，而不是生成可能被拆分或重解释的命令。
fn quote_executable(executable: &str) -> Option<String> {
    if executable.is_empty() || executable.contains(['\0', '"']) {
        None
    } else {
        Some(format!("\"{executable}\""))
    }
}

fn is_command_whitespace(character: char) -> bool {
    matches!(character, ' ' | '\t')
}

/// 判断旧插件的共享 `FloePod` 值是否确实启动当前可执行文件。
///
/// 旧插件对含空格路径不加引号，因此先允许“整个值就是 exe 路径”的旧格式；
/// 其余格式只接受 Windows 能明确识别的首个 quoted/unquoted executable token。
/// 无法消除歧义的命令保持原样，绝不替其他实例删除。
fn legacy_value_targets_executable(command: &str, normalized_executable: &str) -> bool {
    if command.contains('\0') {
        return false;
    }
    let command = command.trim_matches(is_command_whitespace);
    if normalize_windows_path_key(command).as_deref() == Some(normalized_executable) {
        return true;
    }

    let candidate = if let Some(quoted) = command.strip_prefix('"') {
        let Some(closing_quote) = quoted.find('"') else {
            return false;
        };
        let remainder = &quoted[closing_quote + 1..];
        if !remainder.is_empty() && !remainder.chars().next().is_some_and(is_command_whitespace) {
            return false;
        }
        &quoted[..closing_quote]
    } else {
        command
            .split_once(is_command_whitespace)
            .map_or(command, |(executable, _)| executable)
    };

    normalize_windows_path_key(candidate).as_deref() == Some(normalized_executable)
}

#[cfg(windows)]
mod registry {
    use std::io;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    use super::{legacy_value_targets_executable, LEGACY_VALUE_NAME};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const MAX_REGISTRY_VALUE_BYTES: u32 = 64 * 1024;

    struct OwnedKey(HKEY);

    impl Drop for OwnedKey {
        fn drop(&mut self) {
            // SAFETY: OwnedKey is only constructed from a successful registry open/create call
            // and exclusively owns the returned handle.
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    pub(super) fn sync(
        value_name: &str,
        command: &str,
        normalized_executable: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let Some(key) = open_run_key(enabled)? else {
            return Ok(());
        };

        if enabled {
            set_string(&key, value_name, command)?;
        } else {
            delete_value_if_present(&key, value_name)?;
        }

        migrate_legacy_value(&key, normalized_executable)
    }

    fn open_run_key(create: bool) -> Result<Option<OwnedKey>, String> {
        let path = wide_null(RUN_KEY);
        let mut key = null_mut();
        // SAFETY: path is NUL-terminated, output storage is valid, and no security descriptor
        // or class string is supplied. The returned handle is wrapped immediately.
        let status = unsafe {
            if create {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    path.as_ptr(),
                    0,
                    null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_QUERY_VALUE | KEY_SET_VALUE,
                    null(),
                    &mut key,
                    null_mut(),
                )
            } else {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    path.as_ptr(),
                    0,
                    KEY_QUERY_VALUE | KEY_SET_VALUE,
                    &mut key,
                )
            }
        };

        if !create && matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
            return Ok(None);
        }
        check_status(status, "打开 HKCU Run 注册表项")?;
        Ok(Some(OwnedKey(key)))
    }

    fn set_string(key: &OwnedKey, name: &str, value: &str) -> Result<(), String> {
        let name = wide_null(name);
        let data = wide_null(value);
        let byte_len = data
            .len()
            .checked_mul(size_of::<u16>())
            .and_then(|len| u32::try_from(len).ok())
            .ok_or_else(|| "自启动命令过长，无法写入注册表".to_string())?;
        // SAFETY: name and data are NUL-terminated buffers valid for the duration of the call;
        // byte_len describes the complete UTF-16 REG_SZ including its terminator.
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                byte_len,
            )
        };
        check_status(status, "写入实例专属自启动项")
    }

    fn migrate_legacy_value(key: &OwnedKey, normalized_executable: &str) -> Result<(), String> {
        let Some(command) = read_string(key, LEGACY_VALUE_NAME)? else {
            return Ok(());
        };
        if legacy_value_targets_executable(&command, normalized_executable) {
            delete_value_if_present(key, LEGACY_VALUE_NAME)?;
        }
        Ok(())
    }

    fn read_string(key: &OwnedKey, name: &str) -> Result<Option<String>, String> {
        let name = wide_null(name);
        for _ in 0..3 {
            let mut value_type = 0;
            let mut byte_len = 0;
            // SAFETY: value name is NUL-terminated; null data requests the required byte count.
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    name.as_ptr(),
                    null(),
                    &mut value_type,
                    null_mut(),
                    &mut byte_len,
                )
            };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            check_status(status, "读取旧版自启动项")?;
            // 旧插件只写 REG_SZ。其他类型或异常数据不具备足够身份信息，必须保留。
            if value_type != REG_SZ
                || byte_len == 0
                || byte_len > MAX_REGISTRY_VALUE_BYTES
                || byte_len % size_of::<u16>() as u32 != 0
            {
                return Ok(None);
            }

            let mut data = vec![0u16; byte_len as usize / size_of::<u16>()];
            let mut actual_len = byte_len;
            // SAFETY: data owns byte_len initialized bytes and actual_len advertises exactly
            // that capacity. A concurrent growth is handled by ERROR_MORE_DATA and retried.
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    name.as_ptr(),
                    null(),
                    &mut value_type,
                    data.as_mut_ptr().cast(),
                    &mut actual_len,
                )
            };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            if status == ERROR_MORE_DATA {
                continue;
            }
            check_status(status, "读取旧版自启动项内容")?;
            if value_type != REG_SZ
                || actual_len > byte_len
                || actual_len % size_of::<u16>() as u32 != 0
            {
                return Ok(None);
            }

            data.truncate(actual_len as usize / size_of::<u16>());
            while data.last() == Some(&0) {
                data.pop();
            }
            return Ok(String::from_utf16(&data).ok());
        }

        Err("旧版自启动项在读取期间持续变化，已停止迁移".to_string())
    }

    fn delete_value_if_present(key: &OwnedKey, name: &str) -> Result<(), String> {
        let name = wide_null(name);
        // SAFETY: name is a valid NUL-terminated UTF-16 value name and key is owned/open.
        let status = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        if status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            check_status(status, "删除自启动项")
        }
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn check_status(status: u32, action: &str) -> Result<(), String> {
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!(
                "{action}失败（Win32 错误 {status}）: {}",
                io::Error::from_raw_os_error(status as i32)
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_command_is_always_quoted() {
        assert_eq!(
            quote_executable(r"C:\Program Files\FloePod\floe-pod.exe").as_deref(),
            Some(r#""C:\Program Files\FloePod\floe-pod.exe""#)
        );
        assert_eq!(
            quote_executable(r"D:\Portable\FloePod.exe").as_deref(),
            Some(r#""D:\Portable\FloePod.exe""#)
        );
        assert_eq!(quote_executable(""), None);
        assert_eq!(quote_executable("C:\\bad\0path.exe"), None);
        assert_eq!(quote_executable("C:\\bad\"path.exe"), None);
    }

    #[test]
    fn instance_value_name_is_stable_for_equivalent_paths_and_unique_per_instance() {
        let installed = normalize_windows_path_key(r"C:/Program Files/FloePod/./FloePod.exe")
            .expect("installed path");
        let same_installed =
            normalize_windows_path_key(r"\\?\c:\PROGRAM FILES\FloePod\assets\..\FloePod.exe")
                .expect("same installed path");
        let portable =
            normalize_windows_path_key(r"D:\Portable\FloePod\FloePod.exe").expect("portable path");

        assert_eq!(installed, same_installed);
        assert_eq!(
            instance_value_name(&installed),
            instance_value_name(&same_installed)
        );
        assert_ne!(
            instance_value_name(&installed),
            instance_value_name(&portable)
        );
        assert!(instance_value_name(&installed).starts_with(VALUE_PREFIX));
        assert_eq!(
            instance_value_name(&installed).len(),
            VALUE_PREFIX.len() + 32
        );
    }

    #[test]
    fn legacy_value_is_removed_only_when_it_targets_this_executable() {
        let normalized = normalize_windows_path_key(r"C:\Program Files\FloePod\FloePod.exe")
            .expect("normalized executable");

        // tauri-plugin-autostart 2.5.1 / auto-launch 0.5.0 的旧版未引用格式。
        assert!(legacy_value_targets_executable(
            r"C:\Program Files\FloePod\FloePod.exe",
            &normalized
        ));
        assert!(legacy_value_targets_executable(
            r#""c:/program files/floepod/./FLOEPOD.EXE""#,
            &normalized
        ));
        assert!(legacy_value_targets_executable(
            r#""C:\Program Files\FloePod\FloePod.exe" --background"#,
            &normalized
        ));

        assert!(!legacy_value_targets_executable(
            r"D:\Portable\FloePod\FloePod.exe",
            &normalized
        ));
        assert!(!legacy_value_targets_executable(
            r"C:\Program Files\FloePod\FloePod.exe --background",
            &normalized
        ));
        assert!(!legacy_value_targets_executable(
            r#""C:\Program Files\FloePod Evil\FloePod.exe""#,
            &normalized
        ));
        assert!(!legacy_value_targets_executable(
            r"%LOCALAPPDATA%\FloePod\FloePod.exe",
            &normalized
        ));
    }
}
