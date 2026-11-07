use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
static HIT_GEOMETRY_DIRTY: AtomicBool = AtomicBool::new(true);
pub(crate) fn invalidate_hit_geometry() {
    HIT_GEOMETRY_DIRTY.store(true, Ordering::Release);
}

/// How long to sleep before the next cursor sample. A resting cursor is sampled rarely, which
/// is what makes an idle Tuck cheap; the first sample that sees movement returns to the fast
/// rate. Near the island the resting rate stays quick enough for a short move and click.
pub(crate) fn cursor_poll_ms(near: bool, moved: bool) -> u64 {
    match (near, moved) {
        (true, true) => 40,
        (false, true) => 60,
        (true, false) => 100,
        (false, false) => 250,
    }
}

static FULLSCREEN: AtomicBool = AtomicBool::new(false);
static FULLSCREEN_AUTO_HIDE: AtomicBool = AtomicBool::new(true);
static CURSOR_PAUSE_REASONS: AtomicU8 = AtomicU8::new(0);
static CURSOR_GATE: std::sync::LazyLock<(std::sync::Mutex<()>, std::sync::Condvar)> =
    std::sync::LazyLock::new(|| (std::sync::Mutex::new(()), std::sync::Condvar::new()));
pub(crate) const CURSOR_PAUSE_FULLSCREEN: u8 = 1;
pub(crate) const CURSOR_PAUSE_SESSION: u8 = 2;
pub(crate) const CURSOR_PAUSE_SUSPEND: u8 = 4;

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

/// Clamps logical CSS hit dimensions to stay within the 520x288 window canvas: the 500x264
/// open island, the 10px floating gap and the shell spring's overshoot.
pub fn clamp_hit_size(w: u32, h: u32) -> (u32, u32) {
    let bounded_w = w.clamp(10, 520);
    let bounded_h = h.clamp(10, 288);
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

/// Keep fast sampling within 400 CSS pixels, including across negative monitor origins.
fn cursor_near(bounds: &HitBounds, x: f64, y: f64, scale: f64) -> bool {
    let margin = 400.0 * scale;
    is_point_inside(
        &HitBounds {
            left: bounds.left - margin,
            right: bounds.right + margin,
            top: bounds.top - margin,
            bottom: bounds.bottom + margin,
        },
        x,
        y,
    )
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

#[derive(Clone, Copy)]
struct CursorGeometry {
    x: i32,
    y: i32,
    width: u32,
    scale: f64,
}
impl CursorGeometry {
    fn bounds(self, hit: &HitState) -> HitBounds {
        compute_hit_bounds(
            self.x,
            self.y,
            self.width,
            self.scale,
            hit.width.load(Ordering::Acquire),
            hit.height.load(Ordering::Acquire),
            6.0,
        )
    }
}

/// Re-read the cursor on the UI thread: a queued outside sample must never turn
/// click-through back on after the pointer has already entered the island.
fn apply_hit_test(
    win: &WebviewWindow,
    app: AppHandle,
    was_hovered: Arc<AtomicBool>,
    geometry: Option<CursorGeometry>,
    hit: Arc<HitState>,
    _sampled_hovered: bool,
) -> bool {
    let target = win.clone();
    win.run_on_main_thread(move || {
        let _guard = lock_cursor_gate();
        if CURSOR_PAUSE_REASONS.load(Ordering::SeqCst) != 0 {
            return;
        }
        #[cfg(windows)]
        let hovered = {
            let mut point = POINT::default();
            let current = unsafe { GetCursorPos(&mut point) }
                .ok()
                .map(|_| (point.x as f64, point.y as f64));
            let bounds = geometry.map(|geometry| geometry.bounds(&hit));
            hit_at_delivery(bounds.as_ref(), current)
        };
        #[cfg(not(windows))]
        let hovered = _sampled_hovered;
        // Tauri/Wry and Tao execute this setter inline when already on the event thread.
        let _ = target.set_ignore_cursor_events(!hovered);
        if was_hovered.swap(hovered, Ordering::AcqRel) != hovered {
            let _ = app.emit("hover", hovered);
        }
    })
    .is_ok()
}

fn hit_at_delivery(bounds: Option<&HitBounds>, current: Option<(f64, f64)>) -> bool {
    bounds
        .zip(current)
        .is_some_and(|(bounds, (x, y))| is_point_inside(bounds, x, y))
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
        // Fullscreen apps keep the Windows volume flyout while the island is away.
        crate::osd_flyout::set_fullscreen(hidden);
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

/// Starts adaptive cursor hit testing (see `cursor_poll_ms`). Native lifecycle, display, power and
/// fullscreen events are handled by the hidden window in `lifecycle`.
pub fn start(app: AppHandle, hit_state: Arc<HitState>) {
    let app_cursor = app.clone();
    let hit_cursor = hit_state.clone();

    // Adaptive cursor sampling; fullscreen is handled by lifecycle notifications.
    std::thread::spawn(move || {
        #[cfg(windows)]
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let was_hovered = Arc::new(AtomicBool::new(false));
        let mut near = true;
        let mut moved = true;
        #[cfg(windows)]
        let mut last_cursor: Option<(i32, i32)> = None;
        #[cfg(windows)]
        let mut geometry = None;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(cursor_poll_ms(
                near, moved,
            )));

            let Some(win) = app_cursor.get_webview_window("main") else {
                continue;
            };
            let parked = park_cursor_while_paused(&win, || {
                if was_hovered.swap(false, Ordering::AcqRel) {
                    let _ = app_cursor.emit("hover", false);
                }
            });
            if parked {
                continue;
            }

            // Hit test: physical cursor vs scaled CSS geometry
            let mut is_hovered = false;
            let mut delivery_geometry = None;
            #[cfg(windows)]
            unsafe {
                let mut pt = POINT::default();
                if GetCursorPos(&mut pt).is_ok() {
                    moved = last_cursor.replace((pt.x, pt.y)) != Some((pt.x, pt.y));
                    if geometry.is_none() || HIT_GEOMETRY_DIRTY.swap(false, Ordering::AcqRel) {
                        geometry = win
                            .outer_position()
                            .ok()
                            .zip(win.outer_size().ok())
                            .zip(win.scale_factor().ok());
                    }
                    if let Some(((pos, size), sf)) = geometry {
                        let hit_w = hit_cursor.width.load(Ordering::Relaxed);
                        let hit_h = hit_cursor.height.load(Ordering::Relaxed);
                        let bounds =
                            compute_hit_bounds(pos.x, pos.y, size.width, sf, hit_w, hit_h, 6.0);
                        delivery_geometry = Some(CursorGeometry {
                            x: pos.x,
                            y: pos.y,
                            width: size.width,
                            scale: sf,
                        });
                        is_hovered = is_point_inside(&bounds, pt.x as f64, pt.y as f64);
                        near = cursor_near(&bounds, pt.x as f64, pt.y as f64, sf);
                    }
                }
            }

            // Reconcile even without an enter/leave transition: tray activation changes this externally.
            let _ = apply_hit_test(
                &win,
                app_cursor.clone(),
                was_hovered.clone(),
                delivery_geometry,
                hit_cursor.clone(),
                is_hovered,
            );
        }
    });

    crate::lifecycle::start(app);
}

/// Allowed quick action names opening actual Windows system URI.
pub const ALLOWED_QUICK_ACTIONS: &[&str] = &["capture"];

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
            .inner_size(960.0, 660.0)
            .min_inner_size(640.0, 440.0)
            .resizable(true)
            .focused(true)
            .build()
            .map_err(|e| e.to_string())
            .map(|_| ())
    }
}

/// Opens or focuses settings.html and navigates to the specified view.
/// If `check` is true, sets a query parameter `?check=1` and triggers an update check.
pub fn open_settings_view(app: &AppHandle, view: &str, check: bool) -> Result<(), String> {
    let url_str = if check {
        format!("settings.html?check=1#{view}")
    } else {
        format!("settings.html#{view}")
    };
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
        let _ = win.emit("navigate-settings-view", view);
        if check {
            let _ = win.emit("check-for-updates-requested", ());
        }
        Ok(())
    } else {
        WebviewWindowBuilder::new(app, "settings", WebviewUrl::App(url_str.into()))
            .title("Tuck Settings")
            .inner_size(960.0, 660.0)
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
pub(crate) async fn open_settings_cmd(app: AppHandle, view: Option<String>) -> Result<(), String> {
    // Tray and startup callbacks already run on the event loop. Queue command
    // callers there too, so lookup plus creation is one serialized operation.
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = match view.as_deref() {
            Some("tools") => open_settings_view(&handle, "tools", false),
            _ => open_settings(&handle),
        };
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;
    rx.await
        .map_err(|_| "Settings window unavailable".to_owned())?
}

/// Autostart status check.
#[tauri::command]
pub(crate) async fn get_autostart(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Handle to dynamic tray items for state synchronization.
pub struct TrayItems {
    pub autostart: tauri::menu::CheckMenuItem<tauri::Wry>,
    pub check_updates: tauri::menu::MenuItem<tauri::Wry>,
}

/// Synchronizes the tray menu checkmark with the autostart setting.
pub fn update_tray_autostart(app: &AppHandle, enabled: bool) {
    if let Some(tray) = app.try_state::<TrayItems>() {
        let _ = tray.autostart.set_checked(enabled);
    }
}

/// Autostart enable/disable toggle.
#[tauri::command]
pub(crate) async fn set_autostart(app: AppHandle, enable: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let res = if enable {
        app.autolaunch().enable().map_err(|e| e.to_string())
    } else {
        app.autolaunch().disable().map_err(|e| e.to_string())
    };
    if res.is_ok() {
        update_tray_autostart(&app, enable);
    }
    res
}

/// Pending update metadata stored centrally in Rust.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingUpdateInfo {
    pub version: String,
    pub current_version: String,
    pub body: Option<String>,
    pub date: Option<String>,
}

#[derive(Default)]
pub struct PendingUpdateState(pub Mutex<Option<PendingUpdateInfo>>);

/// Retrieves currently pending update info if available.
#[tauri::command]
pub(crate) fn get_pending_update(
    state: State<'_, PendingUpdateState>,
) -> Option<PendingUpdateInfo> {
    state.0.lock().ok()?.clone()
}

/// Stores the update found by a check, relabels the tray item and tells an open Settings window.
#[tauri::command]
pub(crate) fn set_pending_update(
    app: AppHandle,
    state: State<'_, PendingUpdateState>,
    update: Option<PendingUpdateInfo>,
) -> Result<(), String> {
    if let Ok(mut lock) = state.0.lock() {
        *lock = update.clone();
    }
    set_tray_update_title(&app, update.as_ref().map(|u| u.version.as_str()));
    let _ = app.emit("pending-update-changed", update);
    Ok(())
}

fn set_tray_update_title(app: &AppHandle, version: Option<&str>) {
    if let Some(tray) = app.try_state::<TrayItems>() {
        let text = match version {
            Some(v) => format!("Update available (v{v})"),
            None => "Check for updates".to_string(),
        };
        let _ = tray.check_updates.set_text(text);
    }
}

pub(crate) fn ensure_update_allowed(app: &AppHandle) -> Result<(), String> {
    if let Some(dictation) = app.try_state::<Arc<crate::dictation::DictationService>>() {
        if let Some(reason) = dictation.update_blocker() {
            return Err(reason.into());
        }
    }
    Ok(())
}

/// Refuses to start downloading an update while dictation is busy.
#[tauri::command]
pub(crate) fn check_update_allowed(app: AppHandle) -> Result<(), String> {
    ensure_update_allowed(&app)
}

/// Last check before the installer takes over: stops every service, so a later failure
/// must call `restart_after_failed_update`.
#[tauri::command]
pub(crate) fn prepare_update_install(app: AppHandle) -> Result<(), String> {
    ensure_update_allowed(&app)?;
    crate::run_shutdown_cleanup(&app);
    Ok(())
}

/// `prepare_update_install` stopped the services and the installer did not run. A restart
/// brings them all back.
#[tauri::command]
pub(crate) fn restart_after_failed_update(app: AppHandle) {
    app.restart();
}

#[cfg(test)]
mod idle_cursor_tests {
    use super::*;
    #[test]
    fn queued_reconciliation_uses_latest_css_hit_size() {
        let geometry = CursorGeometry {
            x: 0,
            y: 0,
            width: 520,
            scale: 1.0,
        };
        let hit = HitState::default();
        hit.width.store(500, Ordering::Release);
        assert!(hit_at_delivery(
            Some(&geometry.bounds(&hit)),
            Some((30.0, 20.0))
        ));
        hit.width.store(160, Ordering::Release);
        assert!(!hit_at_delivery(
            Some(&geometry.bounds(&hit)),
            Some((30.0, 20.0))
        ));
    }
    #[test]
    fn delayed_outside_sample_uses_cursor_at_delivery_not_enqueue_time() {
        let bounds = HitBounds {
            left: 100.0,
            right: 200.0,
            top: 0.0,
            bottom: 40.0,
        };
        let queued_outside = (50.0, 20.0);
        assert!(!hit_at_delivery(Some(&bounds), Some(queued_outside)));
        assert!(hit_at_delivery(Some(&bounds), Some((150.0, 20.0))));
        assert!(!hit_at_delivery(Some(&bounds), None));
        assert!(!hit_at_delivery(None, Some((150.0, 20.0))));
    }
    #[test]
    fn fast_region_preserves_hit_bounds_and_scales_on_negative_monitors() {
        let bounds = HitBounds {
            left: -1000.0,
            right: -800.0,
            top: 0.0,
            bottom: 40.0,
        };
        assert!(!is_point_inside(&bounds, -1200.0, 200.0));
        assert!(cursor_near(&bounds, -1200.0, 200.0, 1.5));
        assert!(cursor_near(&bounds, -1600.0, 640.0, 1.5));
        assert!(!cursor_near(&bounds, -1600.1, 640.0, 1.5));
        assert!(!cursor_near(&bounds, -1000.0, 641.0, 1.5));
        assert_eq!(
            cursor_poll_ms(cursor_near(&bounds, -1200.0, 200.0, 1.5), true),
            40
        );
        assert_eq!(
            cursor_poll_ms(cursor_near(&bounds, -1700.0, 200.0, 1.5), true),
            60
        );
    }
    #[test]
    fn a_resting_cursor_is_sampled_less_and_never_slower_near_the_island() {
        assert_eq!(cursor_poll_ms(false, false), 250);
        assert_eq!(cursor_poll_ms(true, false), 100);
        assert!(cursor_poll_ms(true, false) < cursor_poll_ms(false, false));
        assert!(cursor_poll_ms(true, true) < cursor_poll_ms(true, false));
    }
}
