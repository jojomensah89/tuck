use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
static FULLSCREEN: AtomicBool = AtomicBool::new(false);
static FULLSCREEN_AUTO_HIDE: AtomicBool = AtomicBool::new(true);
static CURSOR_PAUSE_REASONS: AtomicU8 = AtomicU8::new(0);
static CURSOR_GATE: std::sync::LazyLock<(std::sync::Mutex<()>, std::sync::Condvar)> =
    std::sync::LazyLock::new(|| (std::sync::Mutex::new(()), std::sync::Condvar::new()));
pub(crate) const CURSOR_PAUSE_FULLSCREEN: u8 = 1;
pub(crate) const CURSOR_PAUSE_SESSION: u8 = 2;
pub(crate) const CURSOR_PAUSE_SUSPEND: u8 = 4;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[cfg(windows)]
use windows::{
    core::HSTRING,
    Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONULL,
            MONITOR_DEFAULTTOPRIMARY,
        },
        System::{
            Com::{CoInitializeEx, COINIT_MULTITHREADED},
            Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
        },
        UI::{
            Shell::{
                SHQueryUserNotificationState, ShellExecuteW, QUNS_BUSY,
                QUNS_RUNNING_D3D_FULL_SCREEN,
            },
            WindowsAndMessaging::{
                GetClassNameW, GetCursorPos, GetDesktopWindow, GetForegroundWindow, GetShellWindow,
                GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindowVisible, IsZoomed,
                SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST,
                SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNORMAL,
                WS_CAPTION, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_THICKFRAME,
            },
        },
    },
};

/// Power state payload matching frontend contract with additive telemetry flags.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PowerState {
    pub percent: u8,
    pub charging: bool,
    pub has_battery: bool,
    pub minutes_left: Option<u32>,
    pub percent_known: bool,
    pub status_known: bool,
    pub ac_connected: Option<bool>,
}

/// Pure helper to parse SYSTEM_POWER_STATUS values.
/// Follows coordinator requirements:
/// - Charging uses ONLY flag 8 (no AC under 100% heuristics).
/// - Unknown percentage reports percent_known: false while preserving numeric DTO percent (100).
/// - Unknown battery status reports status_known: false.
/// - AC connection status is an additive nullable boolean (ac_connected).
pub fn parse_power_status(
    ac_line_status: u8,
    battery_flag: u8,
    battery_life_percent: u8,
    battery_life_time: u32,
) -> PowerState {
    let status_known = battery_flag != 255;
    let has_battery = status_known && (battery_flag & 128) == 0;

    let charging = if has_battery {
        (battery_flag & 8) != 0
    } else {
        false
    };

    let percent_known = battery_life_percent != 255 && battery_life_percent <= 100;
    let percent = if percent_known {
        battery_life_percent
    } else {
        100
    };

    let ac_connected = match ac_line_status {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    };

    let minutes_left = if has_battery
        && !charging
        && battery_life_time != u32::MAX
        && battery_life_time != 0xFFFFFFFF
    {
        Some(battery_life_time / 60)
    } else {
        None
    };

    PowerState {
        percent,
        charging,
        has_battery,
        minutes_left,
        percent_known,
        status_known,
        ac_connected,
    }
}

/// Reads the current system power status from Windows.
pub fn read_power() -> Option<PowerState> {
    #[cfg(windows)]
    unsafe {
        let mut status = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut status).is_ok() {
            Some(parse_power_status(
                status.ACLineStatus,
                status.BatteryFlag,
                status.BatteryLifePercent,
                status.BatteryLifeTime,
            ))
        } else {
            None
        }
    }
    #[cfg(not(windows))]
    None
}

/// Hit bounds in physical screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitBounds {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

/// Clamps logical CSS hit dimensions to stay within the 520x230 window canvas.
pub fn clamp_hit_size(w: u32, h: u32) -> (u32, u32) {
    let bounded_w = w.clamp(10, 520);
    let bounded_h = h.clamp(10, 230);
    (bounded_w, bounded_h)
}

/// Pure helper computing physical screen bounding box for cursor hit testing.
/// Centers the hit area horizontally in the window, with configurable slop.
pub fn compute_hit_bounds(
    window_pos_x: i32,
    window_pos_y: i32,
    window_width: u32,
    scale_factor: f64,
    hit_w: u32,
    hit_h: u32,
    slop_px: f64,
) -> HitBounds {
    let (w_css, h_css) = clamp_hit_size(hit_w, hit_h);
    let w_phys = (w_css as f64) * scale_factor;
    let h_phys = (h_css as f64) * scale_factor;
    let slop = slop_px * scale_factor;

    let left = (window_pos_x as f64) + ((window_width as f64) - w_phys) / 2.0 - slop;
    let right = left + w_phys + 2.0 * slop;
    let top = window_pos_y as f64;
    let bottom = top + h_phys + slop;

    HitBounds {
        left,
        right,
        top,
        bottom,
    }
}

/// Checks whether a physical cursor point falls within the given hit bounds.
pub fn is_point_inside(bounds: &HitBounds, x: f64, y: f64) -> bool {
    x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom
}

/// Rectangle geometry for fullscreen classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Pure helper to classify whether a window covers the entire monitor rectangle,
/// indicating true fullscreen on that monitor without inaccurate maximized tolerance heuristics.
/// `is_excluded` marks windows that can cover the monitor without being fullscreen, such as a
/// captioned maximized window while the taskbar auto-hides.
pub fn is_fullscreen_rect(window_rect: Rect, monitor_rect: Rect, is_excluded: bool) -> bool {
    if is_excluded {
        return false;
    }

    // A true fullscreen window must completely cover the monitor rectangle.
    window_rect.left <= monitor_rect.left
        && window_rect.top <= monitor_rect.top
        && window_rect.right >= monitor_rect.right
        && window_rect.bottom >= monitor_rect.bottom
}

/// Global shared hit box state (CSS logical pixels). Default: idle pill 212x32.
pub struct HitState {
    pub width: AtomicU32,
    pub height: AtomicU32,
}

impl Default for HitState {
    fn default() -> Self {
        Self {
            width: AtomicU32::new(212),
            height: AtomicU32::new(32),
        }
    }
}

/// Applies `WS_EX_TOOLWINDOW` and `WS_EX_NOACTIVATE` to make the window invisible in Alt+Tab
/// and prevent taking keyboard focus when clicked.
#[cfg(windows)]
pub fn apply_island_styles(hwnd: HWND) {
    unsafe {
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let new_style = ex_style | (WS_EX_TOOLWINDOW.0 as isize) | (WS_EX_NOACTIVATE.0 as isize);
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
    }
}

/// Dynamically enables or disables `WS_EX_NOACTIVATE` on the island window.
#[cfg(windows)]
pub fn set_island_focusable(hwnd: HWND, focusable: bool) {
    unsafe {
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let new_style = if focusable {
            ex_style & !(WS_EX_NOACTIVATE.0 as isize)
        } else {
            ex_style | (WS_EX_NOACTIVATE.0 as isize)
        };
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
    }
}

/// Centers the Tuck window at the top center of the primary monitor using actual physical coordinates.
/// Only calls `set_position` if coordinates have changed to avoid recursive `WindowEvent::Moved` loops.
pub fn center_on_primary_monitor(win: &WebviewWindow) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(mon) = win
        .primary_monitor()?
        .or_else(|| win.current_monitor().ok().flatten())
    {
        let size = win.outer_size()?;
        let mon_pos = mon.position();
        let mon_size = mon.size();
        let x = mon_pos.x + (mon_size.width as i32 - size.width as i32) / 2;
        let y = mon_pos.y;
        let current_pos = win.outer_position()?;
        if current_pos.x != x || current_pos.y != y {
            win.set_position(tauri::PhysicalPosition::new(x, y))?;
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn reassert_topmost(win: &WebviewWindow) {
    if let Ok(raw) = win.hwnd() {
        unsafe {
            let _ = SetWindowPos(
                HWND(raw.0 as _),
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                // Async: the island belongs to the main thread, and waiting on it here
                // would let lifecycle messages re-enter the hub's window procedure.
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
            );
        }
    }
}

fn lock_cursor_gate() -> std::sync::MutexGuard<'static, ()> {
    CURSOR_GATE
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// Updates a pause reason under the gate mutex so the cursor thread cannot miss the
/// wakeup, and so a hit-test that started before a pause cannot apply its result after it.
pub(crate) fn set_cursor_pause(reason: u8, paused: bool) {
    let _guard = lock_cursor_gate();
    if paused {
        CURSOR_PAUSE_REASONS.fetch_or(reason, Ordering::SeqCst);
    } else {
        CURSOR_PAUSE_REASONS.fetch_and(!reason, Ordering::SeqCst);
    }
    CURSOR_GATE.1.notify_all();
}

/// Parks the cursor thread while any pause reason is set. Before parking it forces
/// click-through from this thread, so that request is queued after any hit-test result
/// this thread sent earlier. `on_park` runs once before waiting. Returns true when parked.
fn park_cursor_while_paused(win: &WebviewWindow, on_park: impl FnOnce()) -> bool {
    let mut guard = lock_cursor_gate();
    if CURSOR_PAUSE_REASONS.load(Ordering::SeqCst) == 0 {
        return false;
    }
    let _ = win.set_ignore_cursor_events(true);
    on_park();
    while CURSOR_PAUSE_REASONS.load(Ordering::SeqCst) != 0 {
        guard = CURSOR_GATE
            .1
            .wait(guard)
            .unwrap_or_else(|error| error.into_inner());
    }
    true
}

/// Applies a hit-test result only if no pause began since the test ran. The window
/// setter only queues a message to the event loop, so holding the gate cannot deadlock.
fn apply_hit_test(win: &WebviewWindow, hovered: bool) -> bool {
    let _guard = lock_cursor_gate();
    if CURSOR_PAUSE_REASONS.load(Ordering::SeqCst) != 0 {
        return false;
    }
    let _ = win.set_ignore_cursor_events(!hovered);
    true
}

#[cfg(windows)]
fn is_excluded_window_class(hwnd: HWND) -> bool {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len > 0 {
        let class_name = String::from_utf16_lossy(&buf[..len as usize]);
        matches!(
            class_name.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        )
    } else {
        false
    }
}

#[cfg(windows)]
pub(crate) fn check_fullscreen(island_hwnd: HWND, settings_hwnd: Option<HWND>) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() || fg == island_hwnd {
            return false;
        }

        if let Some(s_hwnd) = settings_hwnd {
            if fg == s_hwnd {
                return false;
            }
        }

        let desktop = GetDesktopWindow();
        let shell = GetShellWindow();
        if fg == desktop || fg == shell || is_excluded_window_class(fg) {
            return false;
        }

        if !IsWindowVisible(fg).as_bool() || IsIconic(fg).as_bool() {
            return false;
        }

        // Must be on the same monitor as the Tuck island window
        let island_mon: HMONITOR = MonitorFromWindow(island_hwnd, MONITOR_DEFAULTTOPRIMARY);
        let fg_mon: HMONITOR = MonitorFromWindow(fg, MONITOR_DEFAULTTONULL);
        if fg_mon.0.is_null() || fg_mon != island_mon {
            return false;
        }

        // 1. Shell user notification state for apps on the same monitor. Presentation
        // mode is a global user setting, not a property of the foreground window, so it is
        // ignored; slideshows are caught by the size check below.
        let shell_fullscreen = SHQueryUserNotificationState()
            .map(|state| state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_BUSY)
            .unwrap_or(false);
        if shell_fullscreen {
            return true;
        }

        // 2. Window rectangle vs monitor bounds
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(island_mon, &mut mi).as_bool() {
            return false;
        }

        // The DWM frame excludes the invisible resize borders that GetWindowRect includes.
        let mut fg_rect = RECT::default();
        let dwm_ok = DwmGetWindowAttribute(
            fg,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut fg_rect as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok();
        if !dwm_ok && GetWindowRect(fg, &mut fg_rect).is_err() {
            return false;
        }
        // With an auto-hidden taskbar a maximized window covers the whole monitor too.
        // Undecorated apps drop the caption but keep the resize frame, so check both.
        let style = GetWindowLongPtrW(fg, GWL_STYLE) as u32;
        let maximized_with_caption =
            IsZoomed(fg).as_bool() && style & (WS_CAPTION.0 | WS_THICKFRAME.0) != 0;

        let win_rect = Rect {
            left: fg_rect.left,
            top: fg_rect.top,
            right: fg_rect.right,
            bottom: fg_rect.bottom,
        };
        let mon_rect = Rect {
            left: mi.rcMonitor.left,
            top: mi.rcMonitor.top,
            right: mi.rcMonitor.right,
            bottom: mi.rcMonitor.bottom,
        };

        is_fullscreen_rect(win_rect, mon_rect, maximized_with_caption)
    }
}

/// Activates keyboard navigation for the island:
/// Removes `WS_EX_NOACTIVATE`, enables cursor events, focuses, and emits `open-island`.
pub fn open_island(win: &WebviewWindow) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    if let Ok(hwnd) = win.hwnd() {
        set_island_focusable(HWND(hwnd.0 as _), true);
    }
    win.set_ignore_cursor_events(false)?;
    win.show()?;
    win.set_focus()?;
    win.emit("open-island", ())?;
    Ok(())
}

/// Tauri command to trigger island keyboard entry route.
#[tauri::command]
pub(crate) async fn open_island_cmd(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        open_island(&win).map_err(|e| e.to_string())
    } else {
        Err("main window not found".into())
    }
}

/// Cached fullscreen state for listeners that subscribe after native startup.
#[tauri::command]
pub(crate) fn fullscreen_now() -> bool {
    FULLSCREEN.load(Ordering::Relaxed)
}

#[tauri::command]
pub(crate) fn get_fullscreen_auto_hide() -> bool {
    FULLSCREEN_AUTO_HIDE.load(Ordering::Relaxed)
}

#[tauri::command]
pub(crate) fn set_fullscreen_auto_hide(app: AppHandle, enabled: bool) -> Result<(), String> {
    FULLSCREEN_AUTO_HIDE.store(enabled, Ordering::Relaxed);
    // The lifecycle thread owns fullscreen re-evaluation and its follow-up timers.
    if !crate::lifecycle::request_refresh() {
        refresh_window_state(&app);
    }
    Ok(())
}

/// Re-evaluates fullscreen auto-hide. Returns true while the island is hidden for fullscreen.
pub(crate) fn refresh_window_state(app: &AppHandle) -> bool {
    let Some(win) = app.get_webview_window("main") else {
        return false;
    };
    #[cfg(windows)]
    let detected = win
        .hwnd()
        .ok()
        .map(|raw| {
            let settings_hwnd = app
                .get_webview_window("settings")
                .and_then(|window| window.hwnd().ok())
                .map(|handle| HWND(handle.0 as _));
            check_fullscreen(HWND(raw.0 as _), settings_hwnd)
        })
        .unwrap_or(false);
    #[cfg(not(windows))]
    let detected = false;
    let hidden = detected && FULLSCREEN_AUTO_HIDE.load(Ordering::Relaxed);
    let previous = FULLSCREEN.swap(hidden, Ordering::Relaxed);
    set_cursor_pause(CURSOR_PAUSE_FULLSCREEN, hidden);
    if hidden {
        let _ = win.set_ignore_cursor_events(true);
    } else {
        #[cfg(windows)]
        reassert_topmost(&win);
    }
    if hidden != previous {
        let _ = app.emit("fullscreen", hidden);
    }
    hidden
}

/// Collapsing the island must undo the tray keyboard activation path.
#[tauri::command]
pub(crate) fn deactivate_island(app: AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("main")
        .ok_or("Main window unavailable")?;
    #[cfg(windows)]
    if let Ok(hwnd) = win.hwnd() {
        set_island_focusable(HWND(hwnd.0 as _), false);
    }
    win.set_ignore_cursor_events(true)
        .map_err(|_| "Could not restore click-through".to_string())
}

/// Starts the 40ms cursor hit-testing loop. Native lifecycle, display, power and
/// fullscreen events are handled by the hidden window in `lifecycle`.
pub fn start(app: AppHandle, hit_state: Arc<HitState>) {
    let app_cursor = app.clone();
    let hit_cursor = hit_state.clone();

    // 40ms Cursor & Fullscreen polling thread
    std::thread::spawn(move || {
        #[cfg(windows)]
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let mut was_hovered = false;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(40));

            let Some(win) = app_cursor.get_webview_window("main") else {
                continue;
            };
            let parked = park_cursor_while_paused(&win, || {
                if was_hovered {
                    let _ = app_cursor.emit("hover", false);
                }
            });
            if parked {
                was_hovered = false;
                continue;
            }

            // Hit test: physical cursor vs scaled CSS geometry
            let mut is_hovered = false;
            #[cfg(windows)]
            unsafe {
                let mut pt = POINT::default();
                if GetCursorPos(&mut pt).is_ok() {
                    if let (Ok(pos), Ok(size), Ok(sf)) =
                        (win.outer_position(), win.outer_size(), win.scale_factor())
                    {
                        let hit_w = hit_cursor.width.load(Ordering::Relaxed);
                        let hit_h = hit_cursor.height.load(Ordering::Relaxed);
                        let bounds = compute_hit_bounds(
                            pos.x, pos.y, size.width, sf, hit_w, hit_h, 6.0, // 6px slop
                        );
                        is_hovered = is_point_inside(&bounds, pt.x as f64, pt.y as f64);
                    }
                }
            }

            // Reconcile even without an enter/leave transition: tray activation changes this externally.
            if !apply_hit_test(&win, is_hovered) {
                continue;
            }
            if is_hovered != was_hovered {
                was_hovered = is_hovered;
                let _ = app_cursor.emit("hover", is_hovered);
            }
        }
    });

    crate::lifecycle::start(app);
}

/// Allowed quick action names opening actual Windows system URI.
pub const ALLOWED_QUICK_ACTIONS: &[&str] = &["wifi", "bluetooth", "focus", "capture"];

#[cfg(windows)]
fn open_system_uri(uri: &str) -> Result<(), String> {
    let uri_h = HSTRING::from(uri);
    let op_h = HSTRING::from("open");
    unsafe {
        let res = ShellExecuteW(
            HWND(std::ptr::null_mut()),
            &op_h,
            &uri_h,
            None,
            None,
            SW_SHOWNORMAL,
        );
        if (res.0 as isize) > 32 {
            Ok(())
        } else {
            Err(format!(
                "Failed to execute system URI: {uri} (code {})",
                res.0 as isize
            ))
        }
    }
}

#[cfg(not(windows))]
fn open_system_uri(_uri: &str) -> Result<(), String> {
    Ok(())
}

/// Executes a native quick action opening actual Windows settings or screen clip URI.
#[tauri::command]
pub(crate) fn quick_action(name: String) -> Result<(), String> {
    match name.as_str() {
        "wifi" => open_system_uri("ms-settings:network-wifi"),
        "bluetooth" => open_system_uri("ms-settings:bluetooth"),
        "focus" => open_system_uri("ms-settings:quiethours"),
        "capture" => open_system_uri("ms-screenclip:"),
        other => Err(format!(
            "Unknown quick action: '{other}'. Allowed: {}",
            ALLOWED_QUICK_ACTIONS.join(", ")
        )),
    }
}

/// Opens or focuses the reusable 760x600 settings.html window (sidebar layout; 640x440 minimum).
pub fn open_settings(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        win.unminimize().map_err(|e| e.to_string())?;
        win.show().map_err(|e| e.to_string())?;
        win.set_focus().map_err(|e| e.to_string())?;
        Ok(())
    } else {
        WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
            .title("Tuck Settings")
            .inner_size(760.0, 600.0)
            .min_inner_size(640.0, 440.0)
            .resizable(true)
            .focused(true)
            .build()
            .map_err(|e| e.to_string())
            .map(|_| ())
    }
}

/// Frontend command to open settings window without phantom successes.
#[tauri::command]
pub(crate) async fn open_settings_cmd(app: AppHandle) -> Result<(), String> {
    open_settings(&app)
}

/// Autostart status check.
#[tauri::command]
pub(crate) async fn get_autostart(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Autostart enable/disable toggle.
#[tauri::command]
pub(crate) async fn set_autostart(app: AppHandle, enable: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    if enable {
        app.autolaunch().enable().map_err(|e| e.to_string())
    } else {
        app.autolaunch().disable().map_err(|e| e.to_string())
    }
}
