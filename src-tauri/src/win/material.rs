//! Win32 辅助：不抢焦点显示窗口、修饰键状态、前台进程与窗口显隐。

use core::ffi::c_void;

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
