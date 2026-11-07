use tuck_lib::media::{clamp_seek_ms, ms_to_ticks, ticks_to_ms, MediaState};
use tuck_lib::system::{
    clamp_hit_size, compute_hit_bounds, is_fullscreen_rect, is_point_inside, parse_power_status,
    Rect, ALLOWED_QUICK_ACTIONS,
};

#[test]
fn test_hit_bounds_100_percent_dpi() {
    let bounds = compute_hit_bounds(0, 0, 520, 1.0, 212, 32, 6.0);
    assert_eq!(bounds.left, 148.0);
    assert_eq!(bounds.right, 372.0);
    assert_eq!(bounds.top, 0.0);
    assert_eq!(bounds.bottom, 38.0);

    // Inside island
    assert!(is_point_inside(&bounds, 260.0, 16.0));
    // Within 6px slop
    assert!(is_point_inside(&bounds, 150.0, 35.0));
    // Outside horizontally
    assert!(!is_point_inside(&bounds, 100.0, 16.0));
    assert!(!is_point_inside(&bounds, 400.0, 16.0));
    // Outside vertically (below slop)
    assert!(!is_point_inside(&bounds, 260.0, 50.0));
}

#[test]
fn test_hit_bounds_various_dpis() {
    // 125% DPI (sf = 1.25)
    let b125 = compute_hit_bounds(100, 50, 650, 1.25, 212, 32, 6.0);
    assert!(is_point_inside(&b125, 100.0 + 325.0, 50.0 + 20.0));
    assert!(!is_point_inside(&b125, 100.0 + 10.0, 50.0 + 20.0));

    // 150% DPI (sf = 1.5)
    let b150 = compute_hit_bounds(0, 0, 780, 1.5, 300, 38, 6.0);
    assert!(is_point_inside(&b150, 390.0, 25.0));
    assert!(!is_point_inside(&b150, 10.0, 25.0));

    // 200% DPI (sf = 2.0)
    let b200 = compute_hit_bounds(0, 0, 1040, 2.0, 440, 180, 6.0);
    assert!(is_point_inside(&b200, 520.0, 150.0));
    assert!(!is_point_inside(&b200, 520.0, 400.0));
}

#[test]
fn test_hit_bounds_negative_monitor_origin() {
    // Left secondary or negative primary origin
    let bounds = compute_hit_bounds(-1920, -1080, 520, 1.0, 212, 32, 6.0);
    assert_eq!(bounds.left, -1920.0 + 148.0);
    assert_eq!(bounds.right, -1920.0 + 372.0);
    assert_eq!(bounds.top, -1080.0);
    assert_eq!(bounds.bottom, -1080.0 + 38.0);

    assert!(is_point_inside(&bounds, -1920.0 + 260.0, -1080.0 + 16.0));
    assert!(!is_point_inside(&bounds, -1920.0 + 50.0, -1080.0 + 16.0));
}

#[test]
fn test_clamp_hit_size() {
    assert_eq!(clamp_hit_size(0, 0), (10, 10));
    assert_eq!(clamp_hit_size(212, 32), (212, 32));
    assert_eq!(clamp_hit_size(440, 180), (440, 180));
    assert_eq!(clamp_hit_size(1000, 500), (520, 230));
}

#[test]
fn test_fullscreen_rect_classification() {
    let monitor = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };

    // 1. True fullscreen window covering entire monitor
    let fs_win = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };
    assert!(is_fullscreen_rect(fs_win, monitor, false));

    // 2. Maximized window within work area (taskbar remains visible at bottom)
    // Standard maximized window does not reach 1080 bottom
    let max_win = Rect {
        left: -8,
        top: -8,
        right: 1928,
        bottom: 1048,
    };
    assert!(!is_fullscreen_rect(max_win, monitor, false));

    // 3. Maximized window exactly matching work area
    let exact_work = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };
    assert!(!is_fullscreen_rect(exact_work, monitor, false));

    // 4. Normal floating window
    let normal_win = Rect {
        left: 200,
        top: 200,
        right: 1200,
        bottom: 800,
    };
    assert!(!is_fullscreen_rect(normal_win, monitor, false));

    // 5. Excluded window (desktop/shell/settings/island)
    assert!(!is_fullscreen_rect(fs_win, monitor, true));

    // 6. Fullscreen on monitor with negative coordinates
    let neg_monitor = Rect {
        left: -1920,
        top: 0,
        right: 0,
        bottom: 1080,
    };
    let neg_fs = Rect {
        left: -1920,
        top: 0,
        right: 0,
        bottom: 1080,
    };
    assert!(is_fullscreen_rect(neg_fs, neg_monitor, false));
}

#[test]
fn test_maximized_window_with_auto_hidden_taskbar_is_not_fullscreen() {
    let monitor = Rect {
        left: 0,
        top: 0,
        right: 2560,
        bottom: 1440,
    };
    // DWM extended frame bounds of a captioned maximized window when the taskbar
    // auto-hides: identical to the monitor, so only the caption exclusion tells them apart.
    let maximized_frame = monitor;
    assert!(!is_fullscreen_rect(maximized_frame, monitor, true));
    // A borderless fullscreen window covering the same rect is still fullscreen.
    assert!(is_fullscreen_rect(maximized_frame, monitor, false));
    // DWM bounds stop at the visible frame, so a large restored window that only reaches
    // the edges through its invisible resize borders no longer counts.
    let nearly_full = Rect {
        left: 1,
        top: 0,
        right: 2559,
        bottom: 1440,
    };
    assert!(!is_fullscreen_rect(nearly_full, monitor, false));
}

#[test]
fn test_clamp_seek_validation() {
    let duration = Some(180_000); // 3 minutes

    // Clamps negative to 0
    assert_eq!(clamp_seek_ms(-500, duration), 0);
    assert_eq!(clamp_seek_ms(-1, None), 0);

    // Clamps over duration to duration
    assert_eq!(clamp_seek_ms(200_000, duration), 180_000);

    // Within range
    assert_eq!(clamp_seek_ms(45_000, duration), 45_000);

    // Unknown duration allows forward seek without negative
    assert_eq!(clamp_seek_ms(90_000, None), 90_000);
    assert_eq!(clamp_seek_ms(90_000, Some(0)), 90_000);
}

#[test]
fn test_ticks_ms_conversion() {
    assert_eq!(ticks_to_ms(10_000_000), 1_000);
    assert_eq!(ms_to_ticks(1_000), 10_000_000);

    // Saturating math prevents overflow panic
    assert_eq!(ms_to_ticks(i64::MAX), i64::MAX);
}

#[test]
fn test_power_unknown_and_desktop_handling() {
    // 1. Unknown battery flags / desktop unknown
    let unknown = parse_power_status(255, 255, 255, u32::MAX);
    assert!(!unknown.has_battery);
    assert!(!unknown.status_known);
    assert!(!unknown.percent_known);
    assert_eq!(unknown.ac_connected, None);
    assert!(!unknown.charging);
    assert_eq!(unknown.percent, 100);
    assert_eq!(unknown.minutes_left, None);

    // 2. Desktop PC with AC power, no battery (flag 128)
    let desktop = parse_power_status(1, 128, 255, u32::MAX);
    assert!(!desktop.has_battery);
    assert!(desktop.status_known);
    assert!(!desktop.percent_known);
    assert_eq!(desktop.ac_connected, Some(true));
    assert!(!desktop.charging);
    assert_eq!(desktop.percent, 100);
    assert_eq!(desktop.minutes_left, None);

    // 3. Laptop on battery (discharging)
    let laptop_discharging = parse_power_status(0, 1, 75, 5400); // 90 minutes
    assert!(laptop_discharging.has_battery);
    assert!(laptop_discharging.status_known);
    assert!(laptop_discharging.percent_known);
    assert_eq!(laptop_discharging.ac_connected, Some(false));
    assert!(!laptop_discharging.charging);
    assert_eq!(laptop_discharging.percent, 75);
    assert_eq!(laptop_discharging.minutes_left, Some(90));

    // 4. Laptop plugged in charging (flag 8)
    let laptop_charging = parse_power_status(1, 8, 45, u32::MAX);
    assert!(laptop_charging.has_battery);
    assert!(laptop_charging.status_known);
    assert!(laptop_charging.percent_known);
    assert_eq!(laptop_charging.ac_connected, Some(true));
    assert!(laptop_charging.charging);
    assert_eq!(laptop_charging.percent, 45);
    assert_eq!(laptop_charging.minutes_left, None);

    // 5. Laptop plugged in on AC under 100% but NOT flag 8 (should NOT guess charging)
    let laptop_on_ac_not_charging = parse_power_status(1, 1, 80, u32::MAX);
    assert!(laptop_on_ac_not_charging.has_battery);
    assert!(laptop_on_ac_not_charging.status_known);
    assert!(laptop_on_ac_not_charging.percent_known);
    assert_eq!(laptop_on_ac_not_charging.ac_connected, Some(true));
    assert!(!laptop_on_ac_not_charging.charging);
    assert_eq!(laptop_on_ac_not_charging.percent, 80);
    assert_eq!(laptop_on_ac_not_charging.minutes_left, None);

    // 6. Laptop with unknown percentage (255)
    let laptop_unknown_pct = parse_power_status(1, 1, 255, u32::MAX);
    assert!(laptop_unknown_pct.has_battery);
    assert!(laptop_unknown_pct.status_known);
    assert!(!laptop_unknown_pct.percent_known);
    assert_eq!(laptop_unknown_pct.ac_connected, Some(true));
    assert_eq!(laptop_unknown_pct.percent, 100);

    // 7. Percent overflow clamping
    let clamped_pct = parse_power_status(0, 1, 150, 600);
    assert_eq!(clamped_pct.percent, 100);
}

#[test]
fn test_quick_action_allowlist() {
    assert!(!ALLOWED_QUICK_ACTIONS.contains(&"focus"));
    assert!(ALLOWED_QUICK_ACTIONS.contains(&"capture"));

    assert!(!ALLOWED_QUICK_ACTIONS.contains(&"wifi"));
    assert!(!ALLOWED_QUICK_ACTIONS.contains(&"bluetooth"));
    assert!(!ALLOWED_QUICK_ACTIONS.contains(&"cmd"));
    assert!(!ALLOWED_QUICK_ACTIONS.contains(&"powershell"));
    assert!(!ALLOWED_QUICK_ACTIONS.contains(&"malicious"));
    assert!(!ALLOWED_QUICK_ACTIONS.contains(&""));
}

#[test]
fn test_media_state_defaults() {
    let state = MediaState::default();
    assert!(!state.active);
    assert_eq!(state.track_id, "");
    assert_eq!(state.position_ms, 0);
    assert_eq!(state.duration_ms, 0);
    assert_eq!(state.artwork, None);
}
