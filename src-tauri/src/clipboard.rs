//! 系统剪贴板写入（CF_UNICODETEXT / CF_HDROP）。
//! clipboard-manager 插件只支持文本；把文件复制到剪贴板供资源管理器
//! 粘贴必须自行构造 CF_HDROP（DROPFILES + UTF-16 双 NUL 路径列表）。

use std::mem::size_of;
use std::path::Path;
use std::time::Duration;

use windows_sys::Win32::Foundation::{GlobalFree, HGLOBAL, POINT};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::UI::Shell::DROPFILES;

const DROPEFFECT_COPY: u32 = 1;
/// 其他进程持有剪贴板时的重试窗口；失败要有明确报错而不是静默丢动作。
const OPEN_ATTEMPTS: u32 = 6;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(12);

fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// canonicalize 产生的 `\\?\` 前缀在粘贴端（资源管理器）不被识别，复制前剥掉。
fn trim_verbatim(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

/// 打开剪贴板执行写入并确保关闭；`write` 内部调用 SetClipboardData。
fn with_clipboard<T>(write: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let mut open_error = String::new();
    for _ in 0..OPEN_ATTEMPTS {
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                let result = write();
                CloseClipboard();
                return result;
            }
        }
        open_error = "无法打开系统剪贴板（可能被其他程序占用）".into();
        std::thread::sleep(OPEN_RETRY_DELAY);
    }
    Err(open_error)
}

fn alloc_bytes(bytes: &[u8]) -> Result<HGLOBAL, String> {
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if handle.is_null() {
            return Err("剪贴板内存分配失败".into());
        }
        let ptr = GlobalLock(handle);
        if ptr.is_null() {
            GlobalFree(handle);
            return Err("剪贴板内存锁定失败".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast::<u8>(), bytes.len());
        GlobalUnlock(handle);
        Ok(handle)
    }
}

/// 写入一种剪贴板格式。成功后内存归系统所有；失败必须归还，避免泄漏。
fn set_data(format: u16, bytes: &[u8]) -> Result<(), String> {
    unsafe {
        let handle = alloc_bytes(bytes)?;
        if SetClipboardData(format as u32, handle).is_null() {
            GlobalFree(handle);
            return Err("写入剪贴板数据失败".into());
        }
        Ok(())
    }
}

/// 构造 DROPFILES 缓冲：结构头 + 双 NUL 结尾的 UTF-16 路径列表，fWide 标记宽字符。
fn hdrop_buffer(paths: &[&Path]) -> Result<Vec<u8>, String> {
    let mut wide: Vec<u16> = Vec::new();
    for path in paths {
        wide.extend(trim_verbatim(path).encode_utf16());
        wide.push(0);
    }
    if wide.is_empty() {
        return Err("没有可复制的文件".into());
    }
    wide.push(0);
    let header = DROPFILES {
        pFiles: size_of::<DROPFILES>() as u32,
        pt: POINT { x: 0, y: 0 },
        fNC: 0,
        fWide: 1,
    };
    let mut bytes = vec![0u8; size_of::<DROPFILES>()];
    unsafe { std::ptr::write(bytes.as_mut_ptr().cast::<DROPFILES>(), header) };
    bytes.extend_from_slice(unsafe {
        std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2)
    });
    Ok(bytes)
}

/// 把文件 / 文件夹写入剪贴板（CF_HDROP），并附带 Preferred DropEffect=COPY，
/// 避免粘贴端把内容误判为剪切。可在资源管理器直接 Ctrl+V。
pub fn copy_files(paths: &[&Path]) -> Result<(), String> {
    let hdrop = hdrop_buffer(paths)?;
    let effect = DROPEFFECT_COPY.to_le_bytes();
    with_clipboard(|| unsafe {
        EmptyClipboard();
        set_data(CF_HDROP, &hdrop)?;
        let format = RegisterClipboardFormatW(wide_null("Preferred DropEffect").as_ptr());
        if let Ok(format) = u16::try_from(format) {
            // 附加格式写入失败不影响文件已可粘贴。
            let _ = set_data(format, &effect);
        }
        Ok(())
    })
}

/// 把文本写入剪贴板（CF_UNICODETEXT）。
pub fn copy_text(text: &str) -> Result<(), String> {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let bytes =
        unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) }.to_vec();
    with_clipboard(|| unsafe {
        EmptyClipboard();
        set_data(CF_UNICODETEXT, &bytes)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_verbatim_strips_local_and_unc_prefixes() {
        assert_eq!(trim_verbatim(Path::new(r"C:\a\b.txt")), r"C:\a\b.txt");
        assert_eq!(trim_verbatim(Path::new(r"\\?\C:\a\b.txt")), r"C:\a\b.txt");
        assert_eq!(
            trim_verbatim(Path::new(r"\\?\UNC\server\share\a.txt")),
            r"\\server\share\a.txt"
        );
    }

    #[test]
    fn hdrop_buffer_layout_matches_dropfiles_contract() {
        let paths = [Path::new(r"C:\a.txt")];
        let buffer = hdrop_buffer(&paths).unwrap();
        let header = size_of::<DROPFILES>();
        // 头部之后紧跟 UTF-16 路径，双 NUL 结尾。
        let expected: Vec<u16> = r"C:\a.txt".encode_utf16().chain([0]).chain([0]).collect();
        let expected_bytes: Vec<u8> = expected.iter().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(buffer.len(), header + expected_bytes.len());
        assert_eq!(&buffer[header..], expected_bytes.as_slice());
        // fWide 必须置位，粘贴端才知道路径是宽字符。
        assert_eq!(buffer[header - 4], 1);
    }

    #[test]
    fn hdrop_buffer_rejects_empty_list() {
        assert!(hdrop_buffer(&[]).is_err());
    }
}
