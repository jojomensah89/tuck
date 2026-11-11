//! Where the island lives: which monitor, and where along its top edge.
//!
//! The choice is two values, kept in `island-placement.json` in the config directory:
//! the monitor (by a stable identity, never an index) and a dock (left, centre or right).
//! Everything that decides where the window goes is a pure function over plain
//! rectangles, so monitors at negative coordinates and mixed DPI are unit tested. The
//! Win32 enumeration at the bottom only feeds those functions.
//!
//! A monitor that is gone falls back to the primary one, and the island returns to its
//! chosen monitor when it comes back. The choice itself is never rewritten by a fallback.

use crate::system::Rect;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

/// Logical width of the click-through canvas (`src-tauri/tauri.conf.json`). It holds the
/// widest island, the dock inset on one side and the shell spring's overshoot.
pub const CANVAS_WIDTH: u32 = 548;
/// Logical height of the canvas.
pub const CANVAS_HEIGHT: u32 = 288;
/// Logical gap between the screen edge and the island's body when it is docked left or
/// right. It clears the 14px notch ear with air to spare and is `--s-6`; the island's CSS
/// uses the same token (`--dock-inset`), and a test keeps the two together.
pub const DOCK_INSET: u32 = 24;

// The docked canvas sits flush with the screen edge, so it must fit the inset, the widest
// island (500) and the overshoot (about 12) on the other side.
const _: () = assert!(CANVAS_WIDTH >= DOCK_INSET + 500 + 12);

const FILE: &str = "island-placement.json";
const MAX_FILE_BYTES: u64 = 4096;
const MAX_ID_CHARS: usize = 512;
const MAX_NAME_CHARS: usize = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dock {
    Left,
    #[default]
    Center,
    Right,
}

impl Dock {
    fn to_u8(self) -> u8 {
        match self {
            Dock::Left => 0,
            Dock::Center => 1,
            Dock::Right => 2,
        }
    }
    fn from_u8(value: u8) -> Dock {
        match value {
            0 => Dock::Left,
            2 => Dock::Right,
            _ => Dock::Center,
        }
    }
}

/// The monitor the owner picked, remembered by identity and by name. The name is only for
/// showing the choice while the monitor is unplugged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayChoice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Placement {
    /// `None` follows the primary monitor.
    pub display: Option<DisplayChoice>,
    pub dock: Dock,
}

/// One connected monitor, in physical virtual-desktop pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayInfo {
    /// Device path of the monitor, or its GDI device name when Windows gives no path.
    pub id: String,
    /// Friendly name from the monitor's EDID, when there is a usable one.
    pub name: Option<String>,
    /// The `n` of `\\.\DISPLAYn`, for the "Display n" fallback label.
    pub number: Option<u32>,
    pub bounds: Rect,
    /// Effective DPI divided by 96.
    pub scale: f64,
    pub primary: bool,
}

/// What the settings picker needs about one monitor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayEntry {
    pub id: String,
    pub name: String,
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacementState {
    pub display: Option<DisplayChoice>,
    pub dock: Dock,
    pub displays: Vec<DisplayEntry>,
    /// The monitor the island is on right now (the choice, or the primary as a fallback).
    pub active: Option<String>,
}

/// Windows names monitors without an EDID model this way; it tells the owner nothing.
pub fn usable_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("Generic PnP Monitor") {
        None
    } else {
        Some(name.to_owned())
    }
}

/// `\\.\DISPLAY3` is 3.
pub fn gdi_number(device: &str) -> Option<u32> {
    let digits = device.to_ascii_uppercase();
    let digits = digits.strip_prefix(r"\\.\DISPLAY")?;
    digits.parse().ok().filter(|n| *n > 0)
}

/// The name shown in the picker: the friendly name, or "Display n".
pub fn display_label(display: &DisplayInfo, position: usize) -> String {
    display
        .name
        .clone()
        .unwrap_or_else(|| format!("Display {}", display.number.unwrap_or(position as u32 + 1)))
}

/// Picks the monitor for a choice. A remembered identity wins, then a remembered name that
/// only one connected monitor has (a re-plugged monitor can get a new device path), and
/// the primary monitor is the fallback for everything else.
pub fn resolve_display<'a>(
    displays: &'a [DisplayInfo],
    choice: Option<&DisplayChoice>,
) -> Option<&'a DisplayInfo> {
    if let Some(choice) = choice {
        if let Some(found) = displays.iter().find(|display| display.id == choice.id) {
            return Some(found);
        }
        let mut same_name = displays
            .iter()
            .filter(|display| display.name.as_deref() == usable_name(&choice.name).as_deref())
            .filter(|display| display.name.is_some());
        if let (Some(only), None) = (same_name.next(), same_name.next()) {
            return Some(only);
        }
    }
    displays
        .iter()
        .find(|display| display.primary)
        .or_else(|| displays.first())
}

/// The island window for a monitor: the canvas at the monitor's own scale, so the island
/// keeps its physical size on a 150% monitor next to a 100% one. Left and right sit flush
/// with the screen edge (the island's CSS holds the inset); centre is centred. The top is
/// the monitor's top, so negative origins work like any other.
pub fn island_rect(display: &DisplayInfo, dock: Dock) -> Rect {
    let scale = if display.scale.is_finite() && display.scale > 0.0 {
        display.scale
    } else {
        1.0
    };
    let width = (f64::from(CANVAS_WIDTH) * scale).round() as i32;
    let height = (f64::from(CANVAS_HEIGHT) * scale).round() as i32;
    let monitor_width = display.bounds.right - display.bounds.left;
    let left = match dock {
        Dock::Left => display.bounds.left,
        Dock::Right => display.bounds.right - width,
        Dock::Center => display.bounds.left + (monitor_width - width).div_euclid(2),
    };
    Rect {
        left,
        top: display.bounds.top,
        right: left + width,
        bottom: display.bounds.top + height,
    }
}

/// The picker's view of the monitors, in connection order.
pub fn display_entries(displays: &[DisplayInfo]) -> Vec<DisplayEntry> {
    displays
        .iter()
        .enumerate()
        .map(|(position, display)| DisplayEntry {
            id: display.id.clone(),
            name: display_label(display, position),
            primary: display.primary,
        })
        .collect()
}

pub fn state_for(placement: &Placement, displays: &[DisplayInfo]) -> PlacementState {
    PlacementState {
        display: placement.display.clone(),
        dock: placement.dock,
        displays: display_entries(displays),
        active: resolve_display(displays, placement.display.as_ref())
            .map(|display| display.id.clone()),
    }
}

/// A choice from the webview: bounded, and never empty.
pub fn validate_choice(choice: Option<DisplayChoice>) -> Result<Option<DisplayChoice>, String> {
    let Some(choice) = choice else {
        return Ok(None);
    };
    let id = choice.id.trim();
    if id.is_empty() || id.chars().count() > MAX_ID_CHARS {
        return Err("Unknown display".into());
    }
    Ok(Some(DisplayChoice {
        id: id.to_owned(),
        name: choice.name.trim().chars().take(MAX_NAME_CHARS).collect(),
    }))
}

fn parse(bytes: &[u8]) -> Placement {
    serde_json::from_slice(bytes).unwrap_or_default()
}

fn config_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_config_dir().ok().map(|dir| dir.join(FILE))
}

fn load(app: &AppHandle) -> Placement {
    use std::io::Read;
    let Some(path) = config_path(app) else {
        return Placement::default();
    };
    let Ok(file) = std::fs::File::open(path) else {
        return Placement::default();
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_FILE_BYTES
    {
        return Placement::default();
    }
    parse(&bytes)
}

fn save(app: &AppHandle, placement: &Placement) -> Result<(), String> {
    let path = config_path(app).ok_or("Settings directory unavailable")?;
    let dir = path.parent().ok_or("Settings directory unavailable")?;
    std::fs::create_dir_all(dir).map_err(|_| "Could not create settings directory")?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(placement).map_err(|_| "Could not encode the position")?;
    std::fs::write(&temporary, bytes)
        .and_then(|_| std::fs::rename(&temporary, &path))
        .map_err(|_| "Could not save the position".to_owned())
}

static PLACEMENT: Mutex<Option<Placement>> = Mutex::new(None);
static DOCK: AtomicU8 = AtomicU8::new(1);
/// The last state sent to the webviews, so a pass that changed nothing sends nothing.
static PUBLISHED: Mutex<Option<PlacementState>> = Mutex::new(None);

fn lock<T>(mutex: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// Reads the saved choice. Called once at startup, before the first placement.
pub fn init(app: &AppHandle) {
    let placement = load(app);
    DOCK.store(placement.dock.to_u8(), Ordering::Release);
    *lock(&PLACEMENT) = Some(placement);
}

pub fn current() -> Placement {
    lock(&PLACEMENT).clone().unwrap_or_default()
}

/// The dock the hit test uses; one atomic read, no lock.
pub(crate) fn current_dock() -> Dock {
    Dock::from_u8(DOCK.load(Ordering::Acquire))
}

pub fn state() -> PlacementState {
    state_for(&current(), &enumerate())
}

/// Moves the island window to where the saved choice says. Safe to call often and from any
/// thread: it only reads window geometry and posts one asynchronous move when the window is
/// somewhere else, so a pass that finds it in place does nothing.
pub fn apply(win: &WebviewWindow) -> Result<(), Box<dyn std::error::Error>> {
    let placement = current();
    let displays = enumerate();
    let Some(display) = resolve_display(&displays, placement.display.as_ref()) else {
        return Ok(());
    };
    #[allow(unused_mut)]
    let mut target = island_rect(display, placement.dock);
    // The OLE integration probe must never overlap the installed island.
    #[cfg(debug_assertions)]
    if win.app_handle().config().identifier == "com.jojo.tuck.drop-probe" {
        target.top += 400;
        target.bottom += 400;
    }
    move_window(win, target)?;
    DOCK.store(placement.dock.to_u8(), Ordering::Release);
    crate::system::invalidate_hit_geometry();
    publish(win.app_handle(), state_for(&placement, &displays));
    Ok(())
}

fn publish(app: &AppHandle, state: PlacementState) {
    {
        let mut last = lock(&PUBLISHED);
        if last.as_ref() == Some(&state) {
            return;
        }
        *last = Some(state.clone());
    }
    let _ = app.emit("island-placement", state);
}

#[cfg(windows)]
fn move_window(win: &WebviewWindow, target: Rect) -> Result<(), Box<dyn std::error::Error>> {
    use windows::Win32::{
        Foundation::{HWND, RECT},
        UI::WindowsAndMessaging::{
            GetWindowRect, SetWindowPos, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOZORDER,
        },
    };
    let hwnd = HWND(win.hwnd()?.0 as _);
    let mut now = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut now)? };
    let moved = now.left != target.left
        || now.top != target.top
        || now.right - now.left != target.right - target.left
        || now.bottom - now.top != target.bottom - target.top;
    if moved {
        // One call sets position and size together, so a monitor with another scale never
        // sees the old size. Asynchronous: the window belongs to the main thread, and this
        // also runs on the lifecycle thread.
        unsafe {
            SetWindowPos(
                hwnd,
                None,
                target.left,
                target.top,
                target.right - target.left,
                target.bottom - target.top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
            )?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn move_window(win: &WebviewWindow, target: Rect) -> Result<(), Box<dyn std::error::Error>> {
    win.set_size(tauri::PhysicalSize::new(
        (target.right - target.left) as u32,
        (target.bottom - target.top) as u32,
    ))?;
    win.set_position(tauri::PhysicalPosition::new(target.left, target.top))?;
    Ok(())
}

/// The monitor layout or its names may have changed; names are read again on next use.
pub(crate) fn display_config_changed() {
    #[cfg(windows)]
    {
        *lock(&NAMES) = None;
    }
}

#[tauri::command]
pub(crate) fn island_placement_get() -> PlacementState {
    state()
}

#[tauri::command]
pub(crate) fn island_placement_set(
    app: AppHandle,
    display: Option<DisplayChoice>,
    dock: Dock,
) -> Result<PlacementState, String> {
    let next = Placement {
        display: validate_choice(display)?,
        dock,
    };
    save(&app, &next)?;
    *lock(&PLACEMENT) = Some(next);
    if let Some(win) = app.get_webview_window("main") {
        apply(&win).map_err(|_| "Could not move the island".to_owned())?;
    }
    Ok(state())
}

/* ---------------- Win32: the connected monitors ---------------- */

#[cfg(not(windows))]
pub fn enumerate() -> Vec<DisplayInfo> {
    Vec::new()
}

/// GDI device name -> (device path, friendly name).
#[cfg(windows)]
type Names = std::collections::HashMap<String, (String, String)>;
#[cfg(windows)]
static NAMES: Mutex<Option<Names>> = Mutex::new(None);

#[cfg(windows)]
fn wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// Friendly names and device paths from the display configuration. Empty when Windows
/// cannot answer (a remote session, say); monitors are then identified by GDI name.
#[cfg(windows)]
fn read_names() -> Names {
    use windows::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
        DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
        DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
    };
    let mut names = Names::new();
    unsafe {
        let (mut paths, mut modes) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths, &mut modes).is_err() {
            return names;
        }
        let mut path_buffer = vec![DISPLAYCONFIG_PATH_INFO::default(); paths as usize];
        let mut mode_buffer = vec![DISPLAYCONFIG_MODE_INFO::default(); modes as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut paths,
            path_buffer.as_mut_ptr(),
            &mut modes,
            mode_buffer.as_mut_ptr(),
            None,
        )
        .is_err()
        {
            return names;
        }
        for path in path_buffer.iter().take(paths as usize) {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            target.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            target.header.adapterId = path.targetInfo.adapterId;
            target.header.id = path.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut target.header) != 0 {
                continue;
            }
            names.insert(
                wide(&source.viewGdiDeviceName),
                (
                    wide(&target.monitorDevicePath),
                    wide(&target.monitorFriendlyDeviceName),
                ),
            );
        }
    }
    names
}

#[cfg(windows)]
fn names() -> Names {
    let mut cache = lock(&NAMES);
    cache.get_or_insert_with(read_names).clone()
}

/// The connected monitors, in the order Windows lists them.
#[cfg(windows)]
pub fn enumerate() -> Vec<DisplayInfo> {
    use windows::Win32::{
        Foundation::{BOOL, LPARAM, RECT},
        Graphics::Gdi::{
            EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
        },
        UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
    };

    /// `MONITORINFOF_PRIMARY` in winuser.h.
    const MONITORINFOF_PRIMARY: u32 = 1;

    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let found = &mut *(data.0 as *mut Vec<(String, Rect, f64, bool)>);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(
            monitor,
            (&mut info as *mut MONITORINFOEXW).cast::<MONITORINFO>(),
        )
        .as_bool()
        {
            let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
            if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_err() {
                dpi_x = 96;
            }
            let rect = info.monitorInfo.rcMonitor;
            found.push((
                wide(&info.szDevice),
                Rect {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                },
                f64::from(dpi_x.max(96)) / 96.0,
                info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            ));
        }
        BOOL(1)
    }

    let mut found: Vec<(String, Rect, f64, bool)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(collect),
            LPARAM((&mut found as *mut Vec<(String, Rect, f64, bool)>) as isize),
        );
    }
    let names = names();
    found
        .into_iter()
        .map(|(device, bounds, scale, primary)| {
            let (path, friendly) = names.get(&device).cloned().unwrap_or_default();
            DisplayInfo {
                id: if path.is_empty() {
                    device.clone()
                } else {
                    path
                },
                name: usable_name(&friendly),
                number: gdi_number(&device),
                bounds,
                scale,
                primary,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    fn display(
        id: &str,
        name: Option<&str>,
        bounds: Rect,
        scale: f64,
        primary: bool,
    ) -> DisplayInfo {
        DisplayInfo {
            id: id.into(),
            name: name.map(str::to_owned),
            number: None,
            bounds,
            scale,
            primary,
        }
    }

    /// Primary 1080p at 100%, a 4K monitor at 150% to its right, and a 1440p monitor at 125%
    /// that sits left of and above the primary, so its origin is negative on both axes.
    fn desk() -> Vec<DisplayInfo> {
        vec![
            display(
                "main",
                Some("DELL U2723QE"),
                rect(0, 0, 1920, 1080),
                1.0,
                true,
            ),
            display("tv", None, rect(1920, 0, 4800, 1620), 1.5, false),
            display(
                "left",
                Some("LG 27GL"),
                rect(-2560, -300, 0, 1140),
                1.25,
                false,
            ),
        ]
    }

    #[test]
    fn centre_dock_centres_the_canvas_on_the_monitor() {
        let desk = desk();
        assert_eq!(island_rect(&desk[0], Dock::Center), rect(686, 0, 1234, 288));
        // 548 * 1.5 = 822 physical; (2880 - 822) / 2 = 1029 from the left edge.
        assert_eq!(
            island_rect(&desk[1], Dock::Center),
            rect(2949, 0, 3771, 432)
        );
    }

    #[test]
    fn left_and_right_docks_sit_flush_with_the_screen_edge() {
        let desk = desk();
        assert_eq!(island_rect(&desk[0], Dock::Left), rect(0, 0, 548, 288));
        assert_eq!(island_rect(&desk[0], Dock::Right), rect(1372, 0, 1920, 288));
        assert_eq!(island_rect(&desk[1], Dock::Left), rect(1920, 0, 2742, 432));
        assert_eq!(island_rect(&desk[1], Dock::Right), rect(3978, 0, 4800, 432));
    }

    #[test]
    fn a_monitor_left_of_and_above_the_primary_keeps_its_negative_origin() {
        let desk = desk();
        // 548 * 1.25 = 685, 288 * 1.25 = 360
        assert_eq!(
            island_rect(&desk[2], Dock::Left),
            rect(-2560, -300, -1875, 60)
        );
        assert_eq!(island_rect(&desk[2], Dock::Right), rect(-685, -300, 0, 60));
        // (2560 - 685) / 2 = 937 from the left edge
        assert_eq!(
            island_rect(&desk[2], Dock::Center),
            rect(-1623, -300, -938, 60)
        );
    }

    #[test]
    fn the_island_keeps_its_physical_size_on_every_scale() {
        // The same logical canvas is 548 px at 100%, 685 at 125% and 822 at 150%.
        for (scale, width) in [(1.0, 548), (1.25, 685), (1.5, 822), (2.0, 1096)] {
            let monitor = display("m", None, rect(0, 0, 3840, 2160), scale, true);
            let placed = island_rect(&monitor, Dock::Center);
            assert_eq!(placed.right - placed.left, width, "scale {scale}");
            assert_eq!(placed.bottom - placed.top, (288.0 * scale).round() as i32);
        }
    }

    #[test]
    fn a_nonsense_scale_falls_back_to_one() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let monitor = display("m", None, rect(0, 0, 1920, 1080), scale, true);
            assert_eq!(island_rect(&monitor, Dock::Left), rect(0, 0, 548, 288));
        }
    }

    #[test]
    fn a_canvas_wider_than_the_monitor_still_centres() {
        let tiny = display("m", None, rect(100, 50, 400, 350), 1.0, true);
        let placed = island_rect(&tiny, Dock::Center);
        assert_eq!(placed.left, 100 + (300 - 548) / 2);
        assert_eq!(placed.right - placed.left, 548);
    }

    #[test]
    fn no_choice_follows_the_primary_monitor() {
        let desk = desk();
        assert_eq!(resolve_display(&desk, None).unwrap().id, "main");
        // Whichever monitor is primary now, even when it is not first.
        let mut moved = desk.clone();
        moved[0].primary = false;
        moved[2].primary = true;
        assert_eq!(resolve_display(&moved, None).unwrap().id, "left");
    }

    #[test]
    fn a_remembered_monitor_is_found_by_identity_not_position() {
        let mut desk = desk();
        let choice = DisplayChoice {
            id: "tv".into(),
            name: "Living room".into(),
        };
        assert_eq!(resolve_display(&desk, Some(&choice)).unwrap().id, "tv");
        desk.reverse();
        assert_eq!(resolve_display(&desk, Some(&choice)).unwrap().id, "tv");
    }

    #[test]
    fn a_missing_monitor_falls_back_to_primary_and_returns_when_it_does() {
        let mut desk = desk();
        let choice = DisplayChoice {
            id: "tv".into(),
            name: "Living room".into(),
        };
        let unplugged = desk.remove(1);
        assert_eq!(resolve_display(&desk, Some(&choice)).unwrap().id, "main");
        desk.insert(1, unplugged);
        assert_eq!(resolve_display(&desk, Some(&choice)).unwrap().id, "tv");
    }

    #[test]
    fn a_replugged_monitor_with_a_new_path_is_found_by_its_unique_name() {
        let desk = desk();
        let choice = DisplayChoice {
            id: "old-path".into(),
            name: "LG 27GL".into(),
        };
        assert_eq!(resolve_display(&desk, Some(&choice)).unwrap().id, "left");
    }

    #[test]
    fn two_monitors_with_one_name_are_never_guessed() {
        let desk = vec![
            display("a", Some("LG 27GL"), rect(0, 0, 1920, 1080), 1.0, true),
            display("b", Some("LG 27GL"), rect(1920, 0, 3840, 1080), 1.0, false),
        ];
        let choice = DisplayChoice {
            id: "old-path".into(),
            name: "LG 27GL".into(),
        };
        assert_eq!(resolve_display(&desk, Some(&choice)).unwrap().id, "a");
        let unnamed = DisplayChoice {
            id: "old-path".into(),
            name: "Display 2".into(),
        };
        assert_eq!(resolve_display(&desk, Some(&unnamed)).unwrap().id, "a");
    }

    #[test]
    fn no_monitor_at_all_places_nothing() {
        assert!(resolve_display(&[], None).is_none());
    }

    #[test]
    fn labels_use_the_friendly_name_then_the_display_number() {
        let mut desk = desk();
        desk[1].number = Some(2);
        let entries = display_entries(&desk);
        assert_eq!(entries[0].name, "DELL U2723QE");
        assert_eq!(entries[1].name, "Display 2");
        assert_eq!(entries[2].name, "LG 27GL");
        // Without a GDI number the label counts from the position in the list.
        desk[1].number = None;
        assert_eq!(display_entries(&desk)[1].name, "Display 2");
        assert!(entries[0].primary && !entries[1].primary);
    }

    #[test]
    fn generic_names_and_gdi_names_are_read() {
        assert_eq!(usable_name("  LG 27GL  ").as_deref(), Some("LG 27GL"));
        assert_eq!(usable_name("Generic PnP Monitor"), None);
        assert_eq!(usable_name("   "), None);
        assert_eq!(gdi_number(r"\\.\DISPLAY3"), Some(3));
        assert_eq!(gdi_number(r"\\.\display12"), Some(12));
        assert_eq!(gdi_number(r"\\.\DISPLAY0"), None);
        assert_eq!(gdi_number("DISPLAY1"), None);
    }

    #[test]
    fn state_reports_the_monitor_in_use() {
        let desk = desk();
        let placement = Placement {
            display: Some(DisplayChoice {
                id: "gone".into(),
                name: "Old".into(),
            }),
            dock: Dock::Right,
        };
        let state = state_for(&placement, &desk);
        assert_eq!(state.active.as_deref(), Some("main"));
        assert_eq!(
            state.display, placement.display,
            "the choice is not rewritten"
        );
        assert_eq!(state.dock, Dock::Right);
        assert_eq!(state.displays.len(), 3);
    }

    #[test]
    fn old_and_damaged_files_load_with_defaults() {
        assert_eq!(parse(b"{}"), Placement::default());
        assert_eq!(parse(b"not json"), Placement::default());
        assert_eq!(parse(br#"{"dock":"sideways"}"#), Placement::default());
        let saved = parse(br#"{"display":{"id":"tv","name":"TV"},"dock":"left"}"#);
        assert_eq!(saved.dock, Dock::Left);
        assert_eq!(saved.display.unwrap().id, "tv");
        assert_eq!(parse(br#"{"dock":"right"}"#).dock, Dock::Right);
        assert_eq!(Placement::default().dock, Dock::Center);
    }

    #[test]
    fn the_saved_file_round_trips_and_the_dock_atomics_agree() {
        let placement = Placement {
            display: Some(DisplayChoice {
                id: r"\\?\DISPLAY#X".into(),
                name: "X".into(),
            }),
            dock: Dock::Right,
        };
        let bytes = serde_json::to_vec(&placement).unwrap();
        assert_eq!(parse(&bytes), placement);
        for dock in [Dock::Left, Dock::Center, Dock::Right] {
            assert_eq!(Dock::from_u8(dock.to_u8()), dock);
        }
    }

    #[test]
    fn choices_from_the_webview_are_bounded() {
        assert_eq!(validate_choice(None), Ok(None));
        let ok = validate_choice(Some(DisplayChoice {
            id: " tv ".into(),
            name: "n".repeat(500),
        }))
        .unwrap()
        .unwrap();
        assert_eq!(ok.id, "tv");
        assert_eq!(ok.name.chars().count(), MAX_NAME_CHARS);
        assert!(validate_choice(Some(DisplayChoice {
            id: "  ".into(),
            name: "x".into()
        }))
        .is_err());
        assert!(validate_choice(Some(DisplayChoice {
            id: "x".repeat(MAX_ID_CHARS + 1),
            name: "x".into()
        }))
        .is_err());
    }

    /// Reads the real monitors; no state is changed.
    #[cfg(windows)]
    #[test]
    #[ignore = "reads the monitors of the machine running the test"]
    fn this_machines_monitors_enumerate() {
        let displays = enumerate();
        assert!(!displays.is_empty());
        assert_eq!(displays.iter().filter(|d| d.primary).count(), 1);
        for display in &displays {
            eprintln!("{display:?}");
            assert!(display.scale >= 1.0);
            assert!(display.bounds.right > display.bounds.left);
        }
    }
}
