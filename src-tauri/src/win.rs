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
/// 系统材质绘制在整个窗口矩形上，无法跟随 WebView 里的
/// CSS 圆角：不裁剪时材质会在胶囊圆角外露出直角。region 只在材质 != plain 时设置。
/// 区域句柄交给系统接管，无需手动释放；radius <= 0 时清除区域恢复矩形。
/// 贴屏侧通过把圆角矩形延伸出窗口外再由窗口自身裁掉的方式保持直角。
///
/// 每次设置区域前都幂等清理样式与 DWM 非客户区渲染：SetWindowRgn 会触发
/// 系统重算非客户区，窗口样式里残留的 WS_CAPTION / WS_THICKFRAME 位会被
/// 画成旧式标题栏（表现为诡异的「窗口标题」），任何来源恢复的样式位都在
/// 这里被压掉。
pub fn set_bar_region(hwnd: isize, width: i32, height: i32, radius: i32, edge: &str) {
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn};
    prepare_shaped_window(hwnd);
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

/// 把窗口裁剪成四角圆角（radius 为物理像素）的矩形区域，radius <= 0 时清除。
/// 供右键菜单窗口使用：圆角外的透明角落不再吞点击，点击穿透到下层窗口，
/// 菜单随之失焦关闭。
pub fn set_rounded_region(hwnd: isize, width: i32, height: i32, radius: i32) {
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn};
    unsafe {
        let region = if radius <= 0 {
            std::ptr::null_mut()
        } else {
            CreateRoundRectRgn(0, 0, width, height, radius * 2, radius * 2)
        };
        if SetWindowRgn(hwnd as *mut c_void, region, 1) == 0 && !region.is_null() {
            DeleteObject(region);
        }
    }
}

/// 准备「按区域成形」的窗口（胶囊条设置材质时用）。
///
/// 设置窗口区域（SetWindowRgn）会触发系统重算非客户区：窗口样式里残留的
/// WS_CAPTION / WS_THICKFRAME 位、以及 DWM 的非客户区渲染，都会在小小的
/// 胶囊条上画出旧式标题栏 / 边框（表现为诡异的「窗口标题」）。这里在应用
/// 区域之前把这些来源全部去掉：清除样式位并请求 DWM 停止绘制非客户区。
pub fn prepare_shaped_window(hwnd: isize) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
        WS_SYSMENU, WS_THICKFRAME,
    };
    unsafe {
        let hwnd = hwnd as *mut c_void;
        // DWM 不再绘制任何非客户区内容（标题栏 / 边框 / 系统阴影）。
        let policy: i32 = DWMNCRP_DISABLED;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY as u32,
            &policy as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
        // 清掉标题栏相关样式位；胶囊条不可调整大小，移除 WS_THICKFRAME 无副作用。
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let cleaned =
            style & !(WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX);
        if cleaned != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, cleaned as isize);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            );
        }
    }
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

/// 胶囊条系统材质：走 SetWindowCompositionAttribute（SWCA）的 ACCENT 亚克力，
/// 不走 tauri `set_effects` 所用的 DWM systembackdrop。
///
/// 根因：Win11 22H2+ 的 DWMWA_SYSTEMBACKDROP_TYPE 把材质铺在整个窗口矩形上，
/// 不遵循 SetWindowRgn 裁剪出的胶囊区域，材质会在 CSS 圆角外露出直角；
/// SWCA 亚克力作为窗口背景合成，会被窗口 region 正确裁剪，材质与胶囊圆角
/// 完全贴合。云母与亚克力在几十像素高的胶囊条上观感无差别，统一按亚克力
/// 处理以保证任意材质下形状都正确。
pub fn apply_bar_material(hwnd: isize, material: &str) {
    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }
    #[repr(C)]
    struct CompositionAttribData {
        attrib: u32,
        pv_data: *mut c_void,
        cb_data: usize,
    }
    type SetWindowCompositionAttributeFn =
        unsafe extern "system" fn(*mut c_void, *mut CompositionAttribData) -> i32;

    // ACCENT_ENABLE_ACRYLICBLURBEHIND = 4，ACCENT_DISABLED = 0。
    let accent_state = match material {
        "acrylic" | "mica" => 4u32,
        _ => 0u32,
    };
    unsafe {
        let module =
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleA(b"user32.dll\0".as_ptr());
        if module.is_null() {
            return;
        }
        let function = windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            module,
            b"SetWindowCompositionAttribute\0".as_ptr(),
        );
        let Some(function) = function else {
            return;
        };
        let set_attribute: SetWindowCompositionAttributeFn = std::mem::transmute(function);
        // WCA_ACCENT_POLICY = 0x13。GradientColor 为 AABBGGRR：alpha 取 1 让系统
        // 亚克力的模糊完整透出（0 会被系统当作禁用），着色交给 CSS 半透明层。
        let mut policy = AccentPolicy {
            accent_state,
            accent_flags: 0,
            gradient_color: 0x01000000,
            animation_id: 0,
        };
        let mut data = CompositionAttribData {
            attrib: 0x13,
            pv_data: &mut policy as *mut AccentPolicy as *mut c_void,
            cb_data: std::mem::size_of::<AccentPolicy>(),
        };
        set_attribute(hwnd as *mut c_void, &mut data);
    }
}
