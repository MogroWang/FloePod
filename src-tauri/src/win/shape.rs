//! Win32 辅助：不抢焦点显示窗口、修饰键状态、前台进程与窗口显隐。

use core::ffi::c_void;

use super::chrome::prepare_shaped_window;

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

/// 边缘浮动条窗口区域：radius <= 0 时设置与窗口同形的矩形区域（清除历史
/// 圆角裁剪）；radius > 0 时为贴屏侧直角、外侧两角按 radius 物理像素圆角。
///
/// 不能用「清除区域（NULL region）」还原矩形：空的窗口区域不裁剪 Windows 11
/// 在窗口矩形外侧延伸的 1px 系统框架，WebView2 在窗口重新显示 / 获得焦点时
/// 会重新启用非客户区合成，框架与幽灵标题栏随之复现（1.6.0 前偶发的矩形
/// 标题栏残留即源于此）。同形矩形区域把窗口外的框架永久裁掉——区域随窗口
/// 持久存在并裁剪全部子窗口，任何来源恢复的非客户区内容都无法落到屏幕上。
/// 贴屏侧通过把圆角矩形延伸出窗口外再由窗口自身裁掉的方式保持直角。
///
/// 每次设置区域前都幂等清理样式与 DWM 非客户区渲染：SetWindowRgn 会触发
/// 系统重算非客户区，窗口样式里残留的 WS_CAPTION / WS_THICKFRAME 位会被
/// 画成旧式标题栏（表现为诡异的「窗口标题」），任何来源恢复的样式位都在
/// 这里被压掉。
pub fn set_bar_region(hwnd: isize, width: i32, height: i32, radius: i32, edge: &str) -> bool {
    use windows_sys::Win32::Graphics::Gdi::{
        CreateRectRgn, CreateRoundRectRgn, DeleteObject, SetWindowRgn,
    };
    prepare_shaped_window(hwnd);
    let applied = unsafe {
        let region = if radius <= 0 {
            CreateRectRgn(0, 0, width, height)
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
        if region.is_null() {
            return false;
        }
        let applied = SetWindowRgn(hwnd as *mut c_void, region, 1) != 0;
        if !applied {
            DeleteObject(region);
        }
        applied
    };
    // SetWindowRgn 本身会再次触发非客户区计算；区域落地后再刷新一次，避免
    // 初始化或跨显示器重定位时留下刚刚生成的合成残影。
    prepare_shaped_window(hwnd);
    applied
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
        if radius > 0 && region.is_null() {
            return;
        }
        if SetWindowRgn(hwnd as *mut c_void, region, 1) == 0 && !region.is_null() {
            DeleteObject(region);
        }
    }
    prepare_shaped_window(hwnd);
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
