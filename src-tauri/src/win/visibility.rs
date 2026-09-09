//! Win32 辅助：不抢焦点显示窗口、修饰键状态、前台进程与窗口显隐。

use core::ffi::c_void;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE,
    SW_SHOW, SW_SHOWNOACTIVATE,
};

use super::chrome::prepare_shaped_window;

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

/// 浮动条不抢焦点显示。无边框约束由常驻消息处理保证，不再通过
/// 改变宽度并阻塞 UI 线程来尝试清除标题栏残影。
pub fn show_bar_no_activate(hwnd: isize) {
    prepare_shaped_window(hwnd);
    show_no_activate(hwnd);
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
