//! Win32 辅助：无边框窗口的消息级防护与框架清理。
//!
//! 浮动面板 1.7.4 起与边缘浮动条共用同一套「无框架身份」：窗口样式中的
//! 非客户区位（WS_CAPTION 等）在创建后立即清除并由常驻子类拦截重写，
//! WM_NCCALCSIZE 恒返回 0 让客户区铺满整个窗口矩形。DWM 因此不再把
//! 窗口当作有框架窗口——系统强调色窗口边框（设置里「在标题栏和窗口
//! 边框上显示强调色」）对它无处可画，此前聚焦后顶部出现的那条蓝色
//! 边框线在结构上不可能再出现。窗口矩形即面板本体：ACCENT 亚克力
//! 材质覆盖整个窗口并随 DWM 系统圆角（DWMWCP_ROUND）裁出轮廓，与
//! 面板形状贴合；面板不绘制任何外阴影。

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

/// 在窗口所属 UI 线程安装无边框消息防护，早于首次显示。浮动条与浮动
/// 面板共用。
///
/// tao 会从内部 WindowFlags 重新生成 WS_CAPTION 等样式（任何窗口标志
/// 变化都会触发 SetWindowLongW 重写）；只在焦点事件之后清理已经太迟。
/// 这里从源头拦截：WM_STYLECHANGING 过滤最终样式、WM_NCCALCSIZE 恒
/// 返回 0（客户区铺满窗口，tao 的阴影内缩分支也不会再执行）、非客户
/// 区绘制一律禁止。窗口创建时残留的样式位由调用方随后按窗口类型选择
/// prepare_shaped_window（自绘形状窗口，连带 DWM 非客户区渲染策略）或
/// prepare_panel_window（浮动面板，保留 ACCENT 合成）清理。幂等。
pub fn install_borderless_chrome_guard(hwnd: isize) -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let handle = hwnd as *mut c_void;
        if GetWindowThreadProcessId(handle, std::ptr::null_mut()) != GetCurrentThreadId() {
            return false;
        }
        if SetWindowSubclass(
            handle,
            Some(borderless_chrome_proc),
            BAR_CHROME_SUBCLASS_ID,
            0,
        ) == 0
        {
            return false;
        }
    }
    true
}

unsafe extern "system" fn borderless_chrome_proc(
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
        // 客户区铺满整个窗口矩形，窗口不存在任何非客户区缝隙；返回 0
        // 同时短路 tao 对阴影窗口的 inset 内缩（该分支只在本子类不拦截
        // 时才会执行）。
        WM_NCCALCSIZE if wparam != 0 => 0,
        WM_NCPAINT => 0,
        // 由透明 WebView 自绘，不让 GDI 擦背景生成白色占位块。
        WM_ERASEBKGND => 1,
        // 必须继续交给 tao 更新激活/焦点状态，不能直接吞掉该消息。
        // 微软规定 lParam=-1 只禁止 DefWindowProc 重画非客户区。
        WM_NCACTIVATE => DefSubclassProc(hwnd, message, wparam, -1),
        WM_NCDESTROY => {
            RemoveWindowSubclass(hwnd, Some(borderless_chrome_proc), subclass_id);
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
        _ => DefSubclassProc(hwnd, message, wparam, lparam),
    }
}

/// 清除非客户区样式位并压制 DWM 的 1px 外描边与焦点过渡动画，返回样式
/// 位是否在本轮调用中被实际清除。不改动 DWMWA_NCRENDERING_POLICY——
/// 浮动面板的 ACCENT 亚克力模糊作用在整个窗口背景上，禁用非客户区渲染
/// 有误伤窗口合成效果的嫌疑，而样式位清除后 DWM 本就无非客户区可画。
fn clean_frame_bits(hwnd: *mut c_void) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_TRANSITIONS_FORCEDISABLED,
    };
    unsafe {
        // Win11 即使样式位干净也可能给窗口外缘描 1px 边线；显式请求不
        // 绘制。不支持该属性的旧系统会安全地忽略调用失败。
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
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY,
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
        // 自绘形状窗口不依赖 DWM 的任何非客户区效果，禁用是一劳永逸的
        // 兜底；与面板不同，这里没有需要 DWM 继续合成的窗口背景。
        let policy: i32 = DWMNCRP_DISABLED;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY as u32,
            &policy as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
        let stripped = clean_frame_bits(hwnd);

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

/// 准备浮动面板窗口：清除创建时残留的非客户区样式位并压制 DWM 外描边
/// 与焦点过渡，返回是否清理过样式位。
///
/// 与 prepare_shaped_window 的差别只有两点：不禁用 DWM 非客户区渲染
/// （ACCENT 亚克力模糊作用于窗口背景合成，保留策略避免误伤），也不做
/// GDI 强制重绘（面板不设窗口区域，不存在区域触发的重算）。清理过
/// 样式位时补一次框架重算：窗口创建时按带框架样式算好了非客户区布局
/// （标题栏 + 边框内缩），样式位清掉后若不重算，客户区会停留在创建时
/// 的旧布局直到下一次真实 resize——首次显示不依赖这种时序巧合。幂等。
pub fn prepare_panel_window(hwnd: isize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    };
    unsafe {
        let handle = hwnd as *mut c_void;
        let stripped = clean_frame_bits(handle);
        if stripped {
            SetWindowPos(
                handle,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            );
        }
        stripped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_survives_native_style_rewrites_without_swallowing_focus() {
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
            assert!(install_borderless_chrome_guard(window.0 as isize));
            assert!(install_borderless_chrome_guard(window.0 as isize));
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

    /// 模拟 tao 对「无边框 + 系统阴影」窗口的 WM_NCCALCSIZE 顶部内缩
    /// （Win11 100% DPI = 1px）。子类安装顺序决定消息顺序：本模拟器装在
    /// 面板防护之前，真实窗口上 tao 的子类同样晚于防护安装——防护对
    /// WM_NCCALCSIZE 直接返回 0，模拟器根本不应收到消息。
    unsafe extern "system" fn mock_tao_inset_proc(
        hwnd: *mut c_void,
        message: u32,
        wparam: usize,
        lparam: isize,
        _subclass_id: usize,
        _data: usize,
    ) -> isize {
        use windows_sys::Win32::UI::Shell::DefSubclassProc;
        use windows_sys::Win32::UI::WindowsAndMessaging::{NCCALCSIZE_PARAMS, WM_NCCALCSIZE};
        match message {
            WM_NCCALCSIZE if wparam != 0 => {
                let result = DefSubclassProc(hwnd, message, wparam, lparam);
                if result == 0 {
                    let params = &mut *(lparam as *mut NCCALCSIZE_PARAMS);
                    params.rgrc[0].top += 1;
                }
                result
            }
            _ => DefSubclassProc(hwnd, message, wparam, lparam),
        }
    }

    fn assert_client_covers_window(hwnd: *mut c_void) {
        use windows_sys::Win32::Foundation::{POINT, RECT};
        use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect};
        unsafe {
            let mut client: RECT = std::mem::zeroed();
            let mut outer: RECT = std::mem::zeroed();
            let mut origin = POINT { x: 0, y: 0 };
            assert_ne!(GetClientRect(hwnd, &mut client), 0);
            assert_ne!(GetWindowRect(hwnd, &mut outer), 0);
            assert_ne!(ClientToScreen(hwnd, &mut origin), 0);
            let report = format!(
                "outer=({},{},{},{}) client=({},{},{},{}) origin=({},{})",
                outer.left,
                outer.top,
                outer.right,
                outer.bottom,
                client.left,
                client.top,
                client.right,
                client.bottom,
                origin.x,
                origin.y
            );
            assert_eq!(origin.x, outer.left, "client-left mismatch: {report}");
            assert_eq!(origin.y, outer.top, "client-top mismatch: {report}");
            // 客户区在两个方向上覆盖整个窗口：不存在 CSS 够不到的非客户区缝隙。
            assert_eq!(client.top, 0, "client rect must start at 0: {report}");
            assert_eq!(client.left, 0, "client rect must start at 0: {report}");
            assert_eq!(
                client.bottom,
                outer.bottom - outer.top,
                "client height mismatch: {report}"
            );
            assert_eq!(
                client.right,
                outer.right - outer.left,
                "client width mismatch: {report}"
            );
        }
    }

    #[test]
    fn panel_guard_blocks_shadow_inset_and_keeps_client_area_full() {
        use core::ffi::c_void;
        use windows_sys::Win32::UI::Shell::SetWindowSubclass;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER, WS_POPUP,
        };

        struct PanelTestWindow(*mut c_void);
        impl Drop for PanelTestWindow {
            fn drop(&mut self) {
                unsafe { DestroyWindow(self.0) };
            }
        }

        unsafe {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let window = PanelTestWindow(CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP | WS_CAPTION,
                10,
                10,
                380,
                240,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ));
            assert!(!window.0.is_null());

            // 先装模拟器再装防护：防护后安装因此先收到消息，对
            // WM_NCCALCSIZE 直接返回 0，模拟器（同真实窗口上的 tao 子类）
            // 不再有机会加顶部内缩。窗口带 WS_CAPTION 建立时样式也是如此
            // ——防护的首次清理与消息拦截必须能压住它。
            assert_ne!(
                SetWindowSubclass(window.0, Some(mock_tao_inset_proc), 7, 0),
                0
            );
            assert!(install_borderless_chrome_guard(window.0 as isize));
            assert!(install_borderless_chrome_guard(window.0 as isize));
            // 真实改变尺寸（而非仅 FRAMECHANGED，更不能与创建尺寸相同——
            // 无几何变化的 SetWindowPos 不会触发 WM_NCCALCSIZE，布局会停留
            // 在创建时按带框架样式算出的旧值）：确保 WM_NCCALCSIZE 以
            // wParam=TRUE 到达，正是 tao 内缩分支会执行的消息形态。
            assert_ne!(
                SetWindowPos(
                    window.0,
                    std::ptr::null_mut(),
                    10,
                    10,
                    420,
                    300,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                ),
                0
            );
            assert_client_covers_window(window.0);
            drop(window);
        }
    }

    #[test]
    fn prepare_panel_window_clears_styles_idempotently() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetWindowLongPtrW, WS_POPUP,
        };

        unsafe {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let window = CreateWindowExW(
                WS_EX_WINDOWEDGE,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP | WS_CAPTION | WS_SYSMENU,
                10,
                10,
                80,
                60,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!window.is_null());

            // 首轮：脏样式被清干净；再写脏样式并重复调用：仍然干净且不崩
            // （幂等是显示 / 焦点路径反复调用的前提）。
            assert!(prepare_panel_window(window as isize));
            for _ in 0..2 {
                SetWindowLongPtrW(
                    window,
                    GWL_STYLE,
                    (WS_POPUP | NON_CLIENT_STYLE_BITS) as isize,
                );
                SetWindowLongPtrW(window, GWL_EXSTYLE, NON_CLIENT_EX_STYLE_BITS as isize);
                assert!(prepare_panel_window(window as isize));
                assert_eq!(
                    GetWindowLongPtrW(window, GWL_STYLE) as u32 & NON_CLIENT_STYLE_BITS,
                    0
                );
                assert_eq!(
                    GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & NON_CLIENT_EX_STYLE_BITS,
                    0
                );
            }
            DestroyWindow(window);
        }
    }
}
