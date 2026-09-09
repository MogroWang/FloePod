//! Win32 辅助：不抢焦点显示窗口、修饰键状态、前台进程与窗口显隐。

use core::ffi::c_void;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, WS_CAPTION,
    WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_STATICEDGE, WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX,
    WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
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

const BAR_CHROME_SUBCLASS_ID: usize = 0x4650_4241;

/// 在窗口所属 UI 线程安装浮动条的无边框消息处理，早于首次显示。
/// tao 会从内部 WindowFlags 重新生成 WS_CAPTION 等样式；只在焦点事件
/// 之后清理已经太迟。这里阻止样式重新写入，并在原生消息边界禁止绘制
/// 非客户区。仅用于没有系统阴影的浮动条，不用于面板或设置窗口。
pub fn install_bar_chrome_guard(hwnd: isize) -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let handle = hwnd as *mut c_void;
        if GetWindowThreadProcessId(handle, std::ptr::null_mut()) != GetCurrentThreadId() {
            return false;
        }
        if SetWindowSubclass(handle, Some(bar_chrome_proc), BAR_CHROME_SUBCLASS_ID, 0) == 0 {
            return false;
        }
    }
    prepare_shaped_window(hwnd);
    true
}

unsafe extern "system" fn bar_chrome_proc(
    hwnd: *mut c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    subclass_id: usize,
    _data: usize,
) -> isize {
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        STYLESTRUCT, WM_ERASEBKGND, WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCPAINT,
        WM_STYLECHANGING,
    };
    match message {
        WM_STYLECHANGING => {
            // 先让下层处理，再过滤最终样式；保留置顶、可见、拖放等正常标志。
            let result = DefSubclassProc(hwnd, message, wparam, lparam);
            if lparam != 0 {
                let styles = &mut *(lparam as *mut STYLESTRUCT);
                match wparam as i32 {
                    GWL_STYLE => styles.styleNew &= !NON_CLIENT_STYLE_BITS,
                    GWL_EXSTYLE => styles.styleNew &= !NON_CLIENT_EX_STYLE_BITS,
                    _ => {}
                }
            }
            result
        }
        // 两种 NCCALCSIZE 参数形式都保持整个窗口为客户区；不留下标题栏高度。
        WM_NCCALCSIZE | WM_NCPAINT => 0,
        // 浮动条由透明 WebView 自绘，不让 GDI 擦背景生成白色占位块。
        WM_ERASEBKGND => 1,
        // 必须继续交给 tao 更新激活/焦点状态，不能直接吞掉该消息。
        // 微软规定 lParam=-1 只禁止 DefWindowProc 重画非客户区。
        WM_NCACTIVATE => DefSubclassProc(hwnd, message, wparam, -1),
        WM_NCDESTROY => {
            RemoveWindowSubclass(hwnd, Some(bar_chrome_proc), subclass_id);
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
        _ => DefSubclassProc(hwnd, message, wparam, lparam),
    }
}

/// 浮动面板窗口的幂等框架抑制：只关掉 Win11 的 1px 系统外描边与 DWM
/// 的焦点过渡动画。绝不清除窗口样式位、不请求框架重算、不禁用非客户
/// 区渲染。
///
/// tao（Tauri 的窗口层）的无边框架构：WS_CAPTION 等样式位在窗口整个
/// 生命周期常驻（to_window_styles 从不清除它们），无边框完全由子类化的
/// WM_NCCALCSIZE 返回 0 实现；带系统阴影的无边框窗口（本面板）还会把
/// 客户区内缩一圈 frame 厚度，DWM 正是在这圈非客户区里绘制系统阴影
/// ——阴影与样式位共存亡。
///
/// 1.5.0 起曾对面板清除样式位并强刷框架：样式位一掉，DWM 框架连同
/// 阴影一起消失，内缩环变成无人绘制的裸窗口表面，呈现为面板四周的
/// 白色边框。因此这里只压制真正多余的部分——Win11 给带框架窗口外缘
/// 描的 1px 边线（DWMWA_BORDER_COLOR 单独负责，不影响阴影）与焦点
/// 过渡动画（透明 WebView2 场景下会闪出残影）。面板的标题栏伪影在
/// 该架构下本就不可能出现：NCCALCSIZE 内缩后顶部非客户区只有 1-2px，
/// 容不下任何标题栏。
pub fn suppress_panel_frame(hwnd: isize) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_TRANSITIONS_FORCEDISABLED,
    };
    unsafe {
        let hwnd = hwnd as *mut c_void;
        // 只关掉 1px 外描边；系统阴影与 DWMWCP_ROUND 圆角继续绘制。
        // 不支持该属性的旧系统会安全地忽略调用失败。
        let border_color: u32 = DWMWA_COLOR_NONE;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            &border_color as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );

        // 焦点切换时 DWM 对窗口框架做过渡动画，透明 WebView2 场景下会
        // 闪出残影；禁用过渡不影响 CSS 自身动效。
        let transitions_disabled: i32 = 1;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED as u32,
            &transitions_disabled as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

/// 关闭窗口的全部 DWM 非客户区来源：禁用非客户区渲染、禁用框架过渡、
/// 显式不绘制边框，并清掉普通与扩展样式中的非客户区样式位。幂等。
///
/// 仅供无系统阴影的自绘形状窗口（边缘浮动条 / 右键菜单）使用——
/// DWMNCRP_DISABLED 会连系统阴影一起关掉，需要阴影的浮动面板禁用
/// 本函数（见 suppress_panel_frame）。返回样式位是否在本轮调用中被
/// 实际清除：只有真正清除过才需要随后的框架重算与重绘。
///
/// 保留 TOPMOST、TOOLWINDOW、LAYERED 等透明置顶窗口正常运行所需的位。
fn disable_dwm_chrome(hwnd: *mut c_void) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_NCRENDERING_POLICY, DWMWA_TRANSITIONS_FORCEDISABLED,
    };
    unsafe {
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

        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let (cleaned_style, cleaned_ex_style) = strip_non_client_styles(style, ex_style);
        if cleaned_style != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, cleaned_style as isize);
        }
        if cleaned_ex_style != ex_style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, cleaned_ex_style as isize);
        }
        cleaned_style != style || cleaned_ex_style != ex_style
    }
}

/// 准备透明、无边框且按区域成形的窗口（边缘浮动条 / 右键菜单）。
///
/// 设置窗口区域（SetWindowRgn）会触发系统重算非客户区：窗口样式里残留的
/// WS_CAPTION / WS_THICKFRAME 位、以及 DWM 的非客户区渲染，都会在小小的
/// 边缘浮动条上画出旧式标题栏 / 边框（表现为诡异的「窗口标题」）。这里在应用
/// 区域之前把这些来源全部去掉：清除普通与扩展样式位、请求 DWM 停止绘制
/// 非客户区并禁用焦点过渡。只有实际清理了样式时才刷新框架，避免在
/// 焦点与拖动路径反复打断 WebView2。浮动条另外安装常驻消息处理，
/// 从源头阻止样式重新引入；本函数保留给首次清理及右键菜单使用。
pub fn prepare_shaped_window(hwnd: isize) {
    use windows_sys::Win32::Graphics::Gdi::{
        RedrawWindow, RDW_ALLCHILDREN, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    };
    unsafe {
        let hwnd = hwnd as *mut c_void;
        let stripped = disable_dwm_chrome(hwnd);

        // 只有本轮真正清掉样式位时才重刷框架。稳态（样式早已干净）下的
        // SWP_FRAMECHANGED + RedrawWindow 是纯扰动：GDI 重绘不作用于
        // WebView2 的 DirectComposition 表面，反而会在显示 / 焦点路径上
        // 反复打断合成，把「重绘请求」变成偶发的白色矩形残影；样式位没
        // 有变化时框架重算的结果也不会改变。真正清除样式位的那一次则
        // 必须立即重算：SWP_FRAMECHANGED 重发 WM_NCCALCSIZE，让系统按
        // 清理后的样式落定非客户区，不在初始化或跨显示器重定位时留下
        // 刚刚生成的合成残影。
        if stripped {
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
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_guard_survives_native_style_rewrites_without_swallowing_focus() {
        use windows_sys::Win32::Foundation::{POINT, RECT};
        use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
        use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetClientRect, GetWindowRect, SendMessageW,
            SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, WM_ERASEBKGND, WM_NCACTIVATE,
            WM_NCPAINT, WS_EX_ACCEPTFILES, WS_EX_TOOLWINDOW, WS_POPUP,
        };

        #[derive(Default)]
        struct Messages {
            activations: usize,
            painting: usize,
            suppress_paint: bool,
        }
        unsafe extern "system" fn observer(
            hwnd: *mut c_void,
            message: u32,
            wparam: usize,
            lparam: isize,
            _id: usize,
            data: usize,
        ) -> isize {
            let seen = &mut *(data as *mut Messages);
            match message {
                WM_NCACTIVATE => {
                    seen.activations += 1;
                    seen.suppress_paint &= lparam == -1;
                }
                WM_NCPAINT | WM_ERASEBKGND => seen.painting += 1,
                _ => {}
            }
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
        struct TestWindow(*mut c_void);
        impl Drop for TestWindow {
            fn drop(&mut self) {
                unsafe { DestroyWindow(self.0) };
            }
        }

        unsafe {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            // 先声明 observer 状态，确保断言失败时窗口也先于它销毁。
            let mut seen = Messages {
                suppress_paint: true,
                ..Messages::default()
            };
            let window = TestWindow(CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_ACCEPTFILES | WS_EX_WINDOWEDGE,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP | WS_CAPTION | WS_SYSMENU,
                10,
                10,
                44,
                190,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ));
            assert!(!window.0.is_null());
            assert_ne!(
                SetWindowSubclass(window.0, Some(observer), 1, &mut seen as *mut _ as usize),
                0
            );
            assert!(install_bar_chrome_guard(window.0 as isize));
            assert!(install_bar_chrome_guard(window.0 as isize));
            seen.activations = 0;
            seen.painting = 0;
            for (width, height) in [(44, 190), (190, 44), (66, 285), (285, 66)] {
                // 模拟 tao 重新生成样式，以及拖放加宽/方向/DPI 变化后的框架重算。
                SetWindowLongPtrW(
                    window.0,
                    GWL_STYLE,
                    (WS_POPUP | NON_CLIENT_STYLE_BITS) as isize,
                );
                SetWindowLongPtrW(
                    window.0,
                    GWL_EXSTYLE,
                    (WS_EX_TOOLWINDOW | WS_EX_ACCEPTFILES | NON_CLIENT_EX_STYLE_BITS) as isize,
                );
                SetWindowPos(
                    window.0,
                    std::ptr::null_mut(),
                    10,
                    10,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_FRAMECHANGED,
                );
                assert_eq!(
                    GetWindowLongPtrW(window.0, GWL_STYLE) as u32 & NON_CLIENT_STYLE_BITS,
                    0
                );
                assert_eq!(
                    GetWindowLongPtrW(window.0, GWL_EXSTYLE) as u32,
                    WS_EX_TOOLWINDOW | WS_EX_ACCEPTFILES
                );
                let mut client: RECT = std::mem::zeroed();
                let mut outer: RECT = std::mem::zeroed();
                let mut origin = POINT { x: 0, y: 0 };
                assert_ne!(GetClientRect(window.0, &mut client), 0);
                assert_ne!(GetWindowRect(window.0, &mut outer), 0);
                assert_ne!(ClientToScreen(window.0, &mut origin), 0);
                assert_eq!((client.right, client.bottom), (width, height));
                assert_eq!((origin.x, origin.y), (outer.left, outer.top));
                SendMessageW(window.0, WM_NCACTIVATE, 1, 0);
                SendMessageW(window.0, WM_NCACTIVATE, 0, 0);
                SendMessageW(window.0, WM_NCPAINT, 1, 0);
                SendMessageW(window.0, WM_ERASEBKGND, 0, 0);
            }
            assert_eq!(seen.activations, 8);
            assert!(seen.suppress_paint);
            assert_eq!(seen.painting, 0);
            drop(window);
        }
    }

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
