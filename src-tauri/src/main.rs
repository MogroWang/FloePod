// Windows 发布版不显示额外的控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    floe_pod_lib::run()
}
