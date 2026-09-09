use super::auto_block::exe_matches;
use super::geometry::*;
use super::panel::{panel_toggle_action, PanelToggleAction};
use super::*;
use crate::state::{PanelMode, PodRuntime};

fn assert_inside_monitor(rect: (i32, i32, i32, i32), monitor: (i32, i32, i32, i32)) {
    let (x, y, width, height) = rect;
    let (mx, my, mw, mh) = monitor;
    assert!(width > 0 && height > 0);
    assert!(x >= mx);
    assert!(y >= my);
    assert!(x.saturating_add(width) <= mx.saturating_add(mw));
    assert!(y.saturating_add(height) <= my.saturating_add(mh));
}

#[test]
fn logical_panel_size_is_scaled_exactly_once() {
    assert_eq!(scale_logical_px(420, 1.0), 420);
    assert_eq!(scale_logical_px(420, 1.5), 630);
    assert_eq!(scale_logical_px(420, 2.0), 840);
    assert_eq!(scale_logical_px(420, f64::NAN), 420);
}

#[test]
fn oversized_panel_is_clipped_without_invalid_clamp_bounds() {
    let monitor = (0, 0, 1080, 1080);
    let bar = (0, 445, 44, 190);
    let rect = panel_geometry(monitor, bar, "left", 2080, 1800, 1.0);
    assert_eq!(rect.2, 1064);
    assert_eq!(rect.3, 1064);
    assert_inside_monitor(rect, monitor);
}

#[test]
fn panel_geometry_handles_negative_monitor_origins_on_every_edge() {
    let monitor = (-1920, -200, 1920, 1080);
    let bars = [
        ("left", (-1920, 245, 44, 190)),
        ("right", (-44, 245, 44, 190)),
        ("top", (-1055, -200, 190, 44)),
        ("bottom", (-1055, 836, 190, 44)),
    ];
    for (edge, bar) in bars {
        assert_inside_monitor(panel_geometry(monitor, bar, edge, 570, 630, 1.0), monitor);
    }
}

#[test]
fn bar_geometry_scales_all_logical_dimensions_for_target_monitor() {
    let monitor = (1920, 0, 3840, 2160);
    let normal = bar_geometry_for_monitor(monitor, "right", 0.5, false, 2.0, 44, 190);
    let accepting = bar_geometry_for_monitor(monitor, "right", 0.5, true, 2.0, 44, 190);

    assert_eq!(normal, (5672, 890, 88, 380));
    assert_eq!(accepting, (5636, 890, 124, 380));
    assert_inside_monitor(normal, monitor);
    assert_inside_monitor(accepting, monitor);

    // 自定义匣宽度与长度同样只缩放一次，接纳态只在短边基础上加宽固定值。
    let wide = bar_geometry_for_monitor(monitor, "left", 0.5, false, 2.0, 60, 240);
    assert_eq!(wide, (1920, 840, 120, 480));
    let wide_accepting = bar_geometry_for_monitor(monitor, "left", 0.5, true, 2.0, 60, 240);
    assert_eq!(wide_accepting, (1920, 840, 156, 480));
}

#[test]
fn panel_gap_and_margin_scale_with_target_monitor() {
    let monitor = (0, 0, 1920, 1080);
    let bar = (0, 350, 88, 380);
    let rect = panel_geometry(monitor, bar, "left", 840, 840, 2.0);

    assert_eq!(rect, (108, 120, 840, 840));

    let oversized = panel_geometry(monitor, bar, "left", 4000, 4000, 2.0);
    assert_eq!(oversized, (16, 16, 1888, 1048));
    assert_inside_monitor(oversized, monitor);
}

#[test]
fn toggle_during_global_pause_restores_instead_of_closing_logically_open_panel() {
    let runtime = PodRuntime {
        panel_visible: true,
        panel_pinned: true,
        mode: PanelMode::Conflict,
        ..PodRuntime::default()
    };

    assert_eq!(
        panel_toggle_action(false, Some(&runtime)),
        PanelToggleAction::Resume { show_target: false }
    );
    assert_eq!(
        panel_toggle_action(false, None),
        PanelToggleAction::Resume { show_target: true }
    );
    assert_eq!(
        panel_toggle_action(true, Some(&runtime)),
        PanelToggleAction::Hide
    );
}

#[test]
fn foreground_exe_matching_is_name_based_and_case_insensitive() {
    assert!(exe_matches("game.exe", "GAME.EXE"));
    assert!(exe_matches("C:\\Games\\game.exe", "game.exe"));
    assert!(exe_matches("game", "game.exe"));
    assert!(exe_matches("\"My Game.exe\"", "my game.exe"));
    assert!(!exe_matches("game.exe", "other.exe"));
    assert!(!exe_matches("game.exe", "gamelite.exe"));
    assert!(!exe_matches("", "game.exe"));
    assert!(!exe_matches("   ", "game.exe"));
}

#[test]
fn monitor_info_keeps_the_frontend_camel_case_contract() {
    let monitor = MonitorInfo {
        name: "DISPLAY2".into(),
        label: "显示器 2".into(),
        primary: false,
        x: -1920,
        y: 0,
        width: 3840,
        height: 2160,
        scale_factor: 2.0,
    };
    let value = serde_json::to_value(monitor).unwrap();
    assert_eq!(value["scaleFactor"], 2.0);
    assert_eq!(value["width"], 3840);
    assert!(value.get("scale_factor").is_none());
}
