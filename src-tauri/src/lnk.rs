//! .lnk 快捷方式创建（经 WScript.Shell COM，由 PowerShell 承载）。

use std::process::Command;

/// 为每个 (目标, 输出.lnk) 创建快捷方式。
pub fn create_shortcuts(pairs: &[(std::path::PathBuf, std::path::PathBuf)]) -> Result<(), String> {
    if pairs.is_empty() {
        return Ok(());
    }

    // PowerShell 的非终止错误默认仍会返回成功，因此必须显式启用 Stop。
    // 每个快捷方式单独执行，既避免超长命令行，也便于在中途失败时回滚已创建文件。
    let mut created = Vec::new();
    for (target, out) in pairs {
        let t = ps_quote(&target.to_string_lossy());
        let o = ps_quote(&out.to_string_lossy());
        let script = format!(
            "$ErrorActionPreference = 'Stop'; \
             $ws = New-Object -ComObject WScript.Shell; \
             $s = $ws.CreateShortcut({o}); \
             $s.TargetPath = {t}; \
             $s.Save()"
        );
        let result = run_powershell(&script);
        if let Err(error) = result {
            for path in created.iter().rev() {
                let _ = std::fs::remove_file(path);
            }
            // CreateShortcut/Save 可能在报错前已经生成目标，也一并清理。
            let _ = std::fs::remove_file(out);
            return Err(error);
        }
        if !out.is_file() {
            for path in created.iter().rev() {
                let _ = std::fs::remove_file(path);
            }
            return Err(format!("快捷方式创建后未找到输出文件：{}", out.display()));
        }
        created.push(out.clone());
    }
    Ok(())
}

/// 优先用 System32 的绝对路径启动 PowerShell，避免沿 PATH 搜索（含历史遗留的
/// "当前目录优先"解析顺序）；环境异常时回退到 PATH 解析。
fn powershell_program() -> std::path::PathBuf {
    if let Some(windir) = std::env::var_os("WINDIR") {
        let absolute = std::path::PathBuf::from(windir)
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        if absolute.is_file() {
            return absolute;
        }
    }
    std::path::PathBuf::from("powershell")
}

fn run_powershell(script: &str) -> Result<(), String> {
    let mut cmd = Command::new(powershell_program());
    cmd.arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(script);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW，避免闪出控制台
        cmd.creation_flags(0x0800_0000);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("启动 PowerShell 失败: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(format!(
            "创建快捷方式失败（退出码 {:?}）",
            output.status.code()
        ))
    } else {
        Err(format!("创建快捷方式失败：{stderr}"))
    }
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// 从目标文件名推导快捷方式显示名：`报告.docx` -> `报告 - 快捷方式.lnk`
pub fn shortcut_name_for(file_name: &str) -> String {
    format!("{file_name} - 快捷方式.lnk")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_quote_escapes_single_quotes() {
        assert_eq!(ps_quote("C:\\O'Brien\\a.txt"), "'C:\\O''Brien\\a.txt'");
    }

    #[test]
    fn shortcut_name_keeps_original_extension() {
        assert_eq!(shortcut_name_for("报告.docx"), "报告.docx - 快捷方式.lnk");
    }
}
