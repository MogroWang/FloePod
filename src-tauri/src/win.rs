//! Win32 辅助：不抢焦点显示窗口、修饰键状态、前台进程与窗口显隐。

use core::ffi::c_void;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, GWL_STYLE,
    HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE,
    WS_CAPTION, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_STATICEDGE, WS_EX_WINDOWEDGE,
    WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

const NON_CLIENT_STYLE_BITS: u32 =
    WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
const NON_CLIENT_EX_STYLE_BITS: u32 =
    WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_STATICEDGE;

fn strip_non_client_styles(style: u32, ex_style: u32) -> (u32, u32) {
    (
        style & !NON_CLIENT_STYLE_BITS,
        ex_style & !NON_CLIENT_EX_STYLE_BITS,
    )
}

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

/// 当前光标的物理屏幕坐标；失败（权限不足等）返回 None。
/// 隐匿模式用它判断指针是否靠近边缘浮动条。
pub fn cursor_pos() -> Option<(i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut point = POINT { x: 0, y: 0 };
    (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
}

/// SW_SHOWNOACTIVATE 显示 + 无激活置顶：
/// 浮动面板出现时不从用户当前应用抢走键盘焦点。
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
    let raw_hwnd = hwnd;
    let hwnd = raw_hwnd as *mut c_void;
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        // 不带 SWP_NOACTIVATE：让菜单成为前台窗口。
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    }
    // WebView2 在透明无边框窗口显示 / 激活时可能重新显露幽灵标题栏。
    // 显示完成后再强制刷新一次非客户区，不能只依赖创建时的 decorations(false)。
    prepare_shaped_window(raw_hwnd);
}

/// 直接隐藏窗口。
/// 不能走 Tauri 的 `hide()`：它对 WebView2 调用 `SetIsVisible(false)`，
/// 会让顶层窗口重新显示（窗口可见但内容区占位），导致浮动面板"收起后仍在屏幕上"。
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
/// 边缘浮动条等自绘形状的窗口需要：系统圆角会把贴边的圆角矩形裁掉。
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

/// 设置 / 清除窗口的胶囊形区域（贴屏侧直角、外侧两角按 radius 物理像素圆角）。
///
/// 边缘浮动条材质已废弃（固定普通半透明），当前仅在 place_pod_bar 时以 radius=0
/// 调用清除区域：既覆盖旧版本升级后残留的裁剪，也让窗口矩形与 WebView 胶囊
/// 保持同形的历史行为。radius <= 0 时清除区域恢复矩形。
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
    // SetWindowRgn 本身会再次触发非客户区计算；区域落地后再刷新一次，避免
    // 初始化或跨显示器重定位时留下刚刚生成的合成残影。
    prepare_shaped_window(hwnd);
}

/// 把窗口裁剪成四角圆角（radius 为物理像素）的矩形区域，radius <= 0 时清除。
/// 供右键菜单窗口使用：圆角外的透明角落不再吞点击，点击穿透到下层窗口，
/// 菜单随之失焦关闭。
pub fn set_rounded_region(hwnd: isize, width: i32, height: i32, radius: i32) {
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn};
    prepare_shaped_window(hwnd);
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
    prepare_shaped_window(hwnd);
}

/// 准备透明、无边框且按区域成形的窗口（边缘浮动条 / 右键菜单）。
///
/// 设置窗口区域（SetWindowRgn）会触发系统重算非客户区：窗口样式里残留的
/// WS_CAPTION / WS_THICKFRAME 位、以及 DWM 的非客户区渲染，都会在小小的
/// 边缘浮动条上画出旧式标题栏 / 边框（表现为诡异的「窗口标题」）。这里在应用
/// 区域之前把这些来源全部去掉：清除普通与扩展样式位、请求 DWM 停止绘制
/// 非客户区并禁用焦点过渡，最后无条件刷新窗口框架和 WebView 子窗口。
///
/// 必须无条件执行 SWP_FRAMECHANGED / RedrawWindow：透明 WebView2 的幽灵标题栏
/// 属于焦点变化后的合成残影，此时样式位往往已经是正确的；旧实现仅在样式位
/// 发生变化时刷新，所以第二次及之后的焦点切换无法清掉残影。
pub fn prepare_shaped_window(hwnd: isize) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_NCRENDERING_POLICY, DWMWA_TRANSITIONS_FORCEDISABLED,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        RedrawWindow, RDW_ALLCHILDREN, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
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

        // 禁用 DWM 在激活 / 失活时针对透明窗口运行的框架过渡；这些过渡正是
        // WebView2 下方短暂显露幽灵标题栏的常见触发点。CSS 仍负责应用自身动效。
        let transitions_disabled: i32 = 1;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED as u32,
            &transitions_disabled as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );

        // Windows 11 即使 decorations(false) 也可能保留 1px DWM 边框；显式请求
        // 不绘制边框。不支持该属性的旧系统会安全地忽略调用失败。
        let border_color: u32 = DWMWA_COLOR_NONE;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            &border_color as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );

        // 同时清掉普通样式和扩展样式中的非客户区来源；保留 TOPMOST、TOOLWINDOW、
        // LAYERED 等透明置顶窗口正常运行所需的位。
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let (cleaned_style, cleaned_ex_style) = strip_non_client_styles(style, ex_style);
        if cleaned_style != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, cleaned_style as isize);
        }
        if cleaned_ex_style != ex_style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, cleaned_ex_style as isize);
        }

        // 即使样式已正确也必须刷新。SWP_FRAMECHANGED 重新发送 WM_NCCALCSIZE，
        // RedrawWindow 则让 WebView2 子窗口与非客户区立即一起重绘；效果等价于
        // 用户通过轻微调整窗口尺寸清掉幽灵标题栏，但不会改变实际几何尺寸。
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        );
        RedrawWindow(
            hwnd,
            std::ptr::null(),
            std::ptr::null_mut(),
            RDW_INVALIDATE | RDW_FRAME | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
    }
}

/// 请求 Windows 11 使用系统圆角（DWMWCP_ROUND）。
/// 浮动面板窗口：CSS 负责裁切 WebView 内容，这里让原生窗口的
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

/// SWCA（SetWindowCompositionAttribute，未公开 API）写入 ACCENT 策略。
/// 与 DWM systembackdrop 不同，ACCENT 材质随窗口 region 与焦点即时可控，
/// 是浮动面板亚克力「失焦不消失」的关键。
fn set_accent(hwnd: isize, accent_state: u32) -> bool {
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

    unsafe {
        // c"" 字面量给出 *const c_char，windows-sys 的 PCSTR 是 *const u8，cast 对齐。
        let module = windows_sys::Win32::System::LibraryLoader::GetModuleHandleA(
            c"user32.dll".as_ptr().cast(),
        );
        if module.is_null() {
            return false;
        }
        let function = windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            module,
            c"SetWindowCompositionAttribute".as_ptr().cast(),
        );
        let Some(function) = function else {
            return false;
        };
        let set_attribute: SetWindowCompositionAttributeFn = std::mem::transmute(function);
        // WCA_ACCENT_POLICY = 0x13。GradientColor 为 AABBGGRR：alpha 取 1 让系统
        // 模糊完整透出（0 会被系统当作禁用），着色交给 CSS 半透明层。
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
        set_attribute(hwnd as *mut c_void, &mut data) != 0
    }
}

/// 清除窗口上的 ACCENT 材质策略（ACCENT_DISABLED）。
/// 从亚克力切回普通时必须显式调用，否则 SWCA 效果残留。
pub fn disable_accent(hwnd: isize) {
    let _ = set_accent(hwnd, 0);
}

/// 浮动面板亚克力材质：恒定下发全量亚克力（ACCENT_ENABLE_ACRYLICBLURBEHIND）。
/// 聚焦与失焦使用同一份策略，不做任何降级替换。
///
/// 走 SWCA 而不是 tauri set_effects 的 DWM systembackdrop：后者在窗口
/// 失焦后直接移除整个 backdrop（此前「不聚焦就看不到材质」的根源），
/// 且重放属性无效；SWCA 亚克力随 ACCENT 策略常驻窗口，配合焦点变化时
/// 的幂等重放，浮动面板无论是否持有焦点材质都保持已下发状态。
pub fn apply_panel_acrylic(hwnd: isize) -> bool {
    // ACCENT_ENABLE_ACRYLICBLURBEHIND = 4
    set_accent(hwnd, 4)
}

/// 重绘窗口及其全部子窗口（WebView2 内容）的 GDI 表面。
/// ACCENT 材质策略写入后 WebView2 的 DirectComposition 表面不一定立即
/// 重新呈现；这里先做 GDI 层重绘，配合调用方的尺寸轻推共同保证材质
/// 应用后立刻可见。
pub fn redraw_window(hwnd: isize) {
    use windows_sys::Win32::Graphics::Gdi::{
        RedrawWindow, RDW_ALLCHILDREN, RDW_INVALIDATE, RDW_UPDATENOW,
    };
    unsafe {
        RedrawWindow(
            hwnd as *mut c_void,
            std::ptr::null(),
            std::ptr::null_mut(),
            RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stripping_non_client_styles_preserves_transparency_and_tool_window_bits() {
        // 这些保留位分别模拟 WS_VISIBLE、WS_EX_LAYERED、WS_EX_TOOLWINDOW。
        let preserved_style = 0x1000_0000;
        let preserved_ex_style = 0x0008_0000 | 0x0000_0080;
        let style = preserved_style | NON_CLIENT_STYLE_BITS;
        let ex_style = preserved_ex_style | NON_CLIENT_EX_STYLE_BITS;

        let (cleaned_style, cleaned_ex_style) = strip_non_client_styles(style, ex_style);

        assert_eq!(cleaned_style, preserved_style);
        assert_eq!(cleaned_ex_style, preserved_ex_style);
    }
}
