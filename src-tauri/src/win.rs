//! Win32 辅助：不抢焦点显示窗口、修饰键状态、前台进程与窗口显隐。

use core::ffi::c_void;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE,
    SW_SHOW, SW_SHOWNOACTIVATE,
};

fn key_down(vk: u16) -> bool {
    // GetAsyncKeyState 返回 i16，高位为 1（负数）即按下
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifierState {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

pub fn modifier_state() -> ModifierState {
    ModifierState {
        ctrl: key_down(VK_CONTROL),
        shift: key_down(VK_SHIFT),
        alt: key_down(VK_MENU),
    }
}

/// SW_SHOWNOACTIVATE 显示 + 无激活置顶：
/// 面板出现时不从用户当前应用抢走键盘焦点。
pub fn show_no_activate(hwnd: isize) {
    let hwnd = hwnd as *mut c_void;
    unsafe {
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        );
    }
}

/// SW_SHOW 显示并激活窗口 + 恢复置顶（右键菜单需要前台焦点才能用 blur 检测外部点击）。
/// 必须与 hide_window 一样直接走 ShowWindow：Tauri 的 show() 会同步 WebView2
/// 的可见性状态，与原生 SW_HIDE 路径混用时透明窗口内容停留在未恢复的合成状态，
/// 菜单第二次起就再也显示不出来。
pub fn show_window(hwnd: isize) {
    let hwnd = hwnd as *mut c_void;
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        // 不带 SWP_NOACTIVATE：让菜单成为前台窗口。
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    }
}

/// 直接隐藏窗口。
/// 不能走 Tauri 的 `hide()`：它对 WebView2 调用 `SetIsVisible(false)`，
/// 会让顶层窗口重新显示（窗口可见但内容区占位），导致面板"收起后仍在屏幕上"。
pub fn hide_window(hwnd: isize) {
    let hwnd = hwnd as *mut c_void;
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
    }
}

/// 当前前台窗口所属进程的可执行文件名（如 "game.exe"）。
/// 供「自动屏蔽」匹配用户配置的应用；拿不到（权限不足、系统进程等）返回 None。
pub fn foreground_exe() -> Option<String> {
    use std::path::Path;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&full)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }
}

/// 禁用 Windows 11 的系统窗口圆角（DWMWCP_DONOTROUND）。
/// 胶囊条等自绘形状的窗口需要：系统圆角会把贴边的圆角矩形裁掉。
pub fn disable_rounding(hwnd: isize) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    };
    unsafe {
        let pref: i32 = DWMWCP_DONOTROUND;
        DwmSetWindowAttribute(
            hwnd as *mut c_void,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &pref as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

/// 把窗口裁剪成「贴屏侧直角、外侧两角按 radius（物理像素）圆角」的胶囊区域。
///
/// 系统材质（亚克力/云母/模糊）绘制在整个窗口矩形上，无法跟随 WebView 里的
/// CSS 圆角：不裁剪时材质会在胶囊圆角外露出直角。region 只在材质 != plain 时设置。
/// 区域句柄交给系统接管，无需手动释放；radius <= 0 时清除区域恢复矩形。
/// 贴屏侧通过把圆角矩形延伸出窗口外再由窗口自身裁掉的方式保持直角。
pub fn set_bar_region(hwnd: isize, width: i32, height: i32, radius: i32, edge: &str) {
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn};
    unsafe {
        let region = if radius <= 0 {
            std::ptr::null_mut()
        } else {
            let (left, top, right, bottom) = match edge {
                "left" => (-radius, 0, width, height),
                "right" => (0, 0, width + radius, height),
                "top" => (0, -radius, width, height),
                _ => (0, 0, width, height + radius),
            };
            CreateRoundRectRgn(left, top, right, bottom, radius * 2, radius * 2)
        };
        // 返回 0 表示失败，此时需自行释放区域避免泄漏。
        if SetWindowRgn(hwnd as *mut c_void, region, 1) == 0 && !region.is_null() {
            DeleteObject(region);
        }
    }
}

/// Windows 构建号（Win10 为 1904x，Win11 ≥ 22000）。读取失败返回 0。
pub fn windows_build() -> u32 {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    static CACHE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| unsafe {
        let subkey = wide("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
        let value = wide("CurrentBuildNumber");
        let mut buf = [0u16; 16];
        let mut size = (buf.len() * std::mem::size_of::<u16>()) as u32;
        let status = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        );
        if status != 0 {
            return 0;
        }
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
            .trim()
            .parse()
            .unwrap_or(0)
    })
}

/// 请求 Windows 11 使用系统圆角（DWMWCP_ROUND）。
/// 面板窗口：CSS 负责裁切 WebView 内容，这里让原生窗口的
/// 阴影/亚克力背景与同一圆角轮廓对齐；旧版 Windows 会忽略不支持的属性。
pub fn prefer_rounded_corners(hwnd: isize) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };
    unsafe {
        let pref: i32 = DWMWCP_ROUND;
        DwmSetWindowAttribute(
            hwnd as *mut c_void,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &pref as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}
