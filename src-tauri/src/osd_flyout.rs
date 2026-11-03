//! Opt-in hiding of the Windows volume/brightness flyout ("OSD").
//!
//! UNDOCUMENTED. Windows has no API to suppress its own flyout. This module uses the
//! technique community tools such as HideVolumeOSD rely on: find the shell window that
//! hosts the flyout and minimize it. A minimized host never paints, so the flyout stays
//! invisible until the window is restored. Everything here is a best-effort guess about
//! Explorer internals and can break with any Windows update:
//!
//! - Windows 10 (builds below 22000): a top-level `NativeHWNDHost` (empty title) with a
//!   `DirectUIHWND` child.
//! - Windows 11 (build 22000 and later): a top-level `XamlExplorerHostIslandWindow`
//!   (empty title) with a `Windows.UI.Composition.DesktopWindowContentBridge` child
//!   titled `DesktopWindowXamlSource`. Explorer uses the same pair for other XAML
//!   surfaces, some of them screen-sized, so candidates that cover their monitor are
//!   skipped and anything still ambiguous counts as "not found".
//!
//! The same host draws the brightness and media-key flyouts, so those are hidden too.
//! Safety rules:
//! - Off by default; only applied while Tuck runs.
//! - Restored on disable and on exit. A marker file records that the flyout is hidden,
//!   so a crash (release builds abort on panic) is repaired on the next start.
//! - If the window cannot be identified, the setting turns itself off and says so.
//! - Explorer restarts create a fresh, visible host; `reapply` re-finds it.
//! - Only `ShowWindowAsync` is used on the foreign window, so a hung Explorer can
//!   never block Tuck.

use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Service preference name (stored as `hide-system-flyout.json`).
pub const PREFERENCE: &str = "hide-system-flyout";
const MARKER: &str = "flyout-hidden.marker";
/// Windows creates the host lazily, on the first flyout after sign-in. While enabled
/// but not found, each volume/brightness change retries; Tuck gives up only after
/// this many failed retries spread over at least `GIVE_UP_AFTER`.
pub(crate) const GIVE_UP_ATTEMPTS: u32 = 3;
pub(crate) const GIVE_UP_AFTER: Duration = Duration::from_secs(60);
/// First build number of Windows 11.
pub(crate) const WINDOWS_11_BUILD: u32 = 22000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FlyoutState {
    /// The setting is off; the Windows flyout behaves normally.
    Off,
    /// The flyout host was found and is minimized.
    Hidden,
    /// Enabled, but Windows has not created the flyout yet; retried on the next change.
    Waiting,
    /// The flyout could not be identified; the setting turned itself off.
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlyoutStatus {
    pub enabled: bool,
    pub state: FlyoutState,
    pub message: Option<String>,
}

impl FlyoutStatus {
    fn new(state: FlyoutState, message: Option<&str>) -> Self {
        Self {
            enabled: matches!(state, FlyoutState::Hidden | FlyoutState::Waiting),
            state,
            message: message.map(str::to_owned),
        }
    }
}

/// Window class and title pairs that identify the flyout host on a given build.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct HostClasses {
    pub outer_class: &'static str,
    pub inner_class: &'static str,
    pub inner_title: &'static str,
}

pub(crate) fn host_classes(build: u32) -> HostClasses {
    if build >= WINDOWS_11_BUILD {
        HostClasses {
            outer_class: "XamlExplorerHostIslandWindow",
            inner_class: "Windows.UI.Composition.DesktopWindowContentBridge",
            inner_title: "DesktopWindowXamlSource",
        }
    } else {
        HostClasses {
            outer_class: "NativeHWNDHost",
            inner_class: "DirectUIHWND",
            inner_title: "",
        }
    }
}

/// A top-level window whose class pair matched.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    /// Owned by the shell (the process of `GetShellWindow`).
    pub shell_owned: bool,
    /// At least as large as its monitor: a full-screen shell surface, not the flyout.
    pub covers_monitor: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pick<T = usize> {
    Found(T),
    NotFound,
    Ambiguous,
}

pub(crate) fn pick_candidate(candidates: &[Candidate]) -> Pick {
    let mut matches = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.shell_owned && !candidate.covers_monitor)
        .map(|(index, _)| index);
    match (matches.next(), matches.next()) {
        (None, _) => Pick::NotFound,
        (Some(index), None) => Pick::Found(index),
        (Some(_), Some(_)) => Pick::Ambiguous,
    }
}

/// Whether a failed retry while waiting should turn the setting off.
pub(crate) fn should_give_up(failed_attempts: u32, waiting_for: Duration) -> bool {
    failed_attempts >= GIVE_UP_ATTEMPTS && waiting_for >= GIVE_UP_AFTER
}

const NOT_FOUND: &str =
    "Couldn't find the Windows flyout on this version of Windows, so it stays visible";
const AMBIGUOUS: &str =
    "Couldn't tell the Windows flyout apart from other shell windows, so it stays visible";
const WAITING: &str = "Takes effect after the next volume or brightness change";

struct Inner {
    app: Option<AppHandle>,
    enabled: bool,
    /// Raw HWND of the minimized host, 0 when not hidden.
    hidden: isize,
    waiting_since: Option<Instant>,
    failed_attempts: u32,
}

static STATE: Mutex<Inner> = Mutex::new(Inner {
    app: None,
    enabled: false,
    hidden: 0,
    waiting_since: None,
    failed_attempts: 0,
});

fn state() -> std::sync::MutexGuard<'static, Inner> {
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Inner {
    fn status(&self) -> FlyoutStatus {
        if !self.enabled {
            FlyoutStatus::new(FlyoutState::Off, None)
        } else if self.hidden != 0 {
            FlyoutStatus::new(FlyoutState::Hidden, None)
        } else {
            FlyoutStatus::new(FlyoutState::Waiting, Some(WAITING))
        }
    }

    fn emit(&self, status: &FlyoutStatus) {
        if let Some(app) = &self.app {
            let _ = app.emit("system-flyout", status);
        }
    }

    fn marker(&self, present: bool) {
        let Some(dir) = self
            .app
            .as_ref()
            .and_then(|app| app.path().app_config_dir().ok())
        else {
            return;
        };
        let path = dir.join(MARKER);
        let result = if present {
            std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, b"1"))
        } else {
            match std::fs::remove_file(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        };
        if let Err(error) = result {
            eprintln!("Flyout marker: {error}");
        }
    }

    fn marker_present(&self) -> bool {
        self.app
            .as_ref()
            .and_then(|app| app.path().app_config_dir().ok())
            .is_some_and(|dir| dir.join(MARKER).exists())
    }

    /// Finds and minimizes the host. Returns a failure status when it turned itself off.
    fn try_hide(&mut self) -> Option<FlyoutStatus> {
        match native::find_host() {
            Pick::Found(hwnd) => {
                native::hide(hwnd);
                self.hidden = hwnd;
                self.waiting_since = None;
                self.failed_attempts = 0;
                self.marker(true);
                None
            }
            Pick::Ambiguous => Some(self.give_up(AMBIGUOUS)),
            Pick::NotFound => {
                let since = *self.waiting_since.get_or_insert_with(Instant::now);
                if should_give_up(self.failed_attempts, since.elapsed()) {
                    Some(self.give_up(NOT_FOUND))
                } else {
                    self.failed_attempts += 1;
                    None
                }
            }
        }
    }

    fn give_up(&mut self, message: &str) -> FlyoutStatus {
        self.restore();
        self.enabled = false;
        if let Some(app) = &self.app {
            if let Err(error) = crate::service_preferences::write(app, PREFERENCE, false) {
                eprintln!("Flyout preference: {error}");
            }
        }
        FlyoutStatus::new(FlyoutState::Unavailable, Some(message))
    }

    fn restore(&mut self) {
        if self.hidden != 0 {
            native::restore(self.hidden);
            self.hidden = 0;
        }
        self.waiting_since = None;
        self.failed_attempts = 0;
        self.marker(false);
    }
}

/// Reads the preference, repairs a flyout left hidden by a crash, then applies.
pub fn start(app: AppHandle) {
    let enabled = crate::service_preferences::read(&app, PREFERENCE);
    let mut inner = state();
    inner.app = Some(app);
    inner.enabled = enabled;
    if inner.marker_present() && !enabled {
        // A previous run hid the flyout and never restored it.
        if let Pick::Found(hwnd) = native::find_host() {
            native::restore(hwnd);
        }
        inner.marker(false);
    }
    if enabled {
        let failure = inner.try_hide();
        let status = failure.unwrap_or_else(|| inner.status());
        inner.emit(&status);
    }
}

pub fn status() -> FlyoutStatus {
    state().status()
}

pub fn set_enabled(app: &AppHandle, enabled: bool) -> Result<FlyoutStatus, String> {
    if !native::SUPPORTED && enabled {
        return Ok(FlyoutStatus::new(
            FlyoutState::Unavailable,
            Some("Only available on Windows"),
        ));
    }
    let mut inner = state();
    if inner.app.is_none() {
        inner.app = Some(app.clone());
    }
    let _write = crate::service_preferences::write(app, PREFERENCE, enabled)?;
    drop(_write);
    inner.enabled = enabled;
    let status = if enabled {
        inner.failed_attempts = 0;
        inner.waiting_since = None;
        inner.try_hide().unwrap_or_else(|| inner.status())
    } else {
        inner.restore();
        inner.status()
    };
    inner.emit(&status);
    Ok(status)
}

/// Called on every volume or brightness change: retries while waiting and re-hides a
/// host that Explorer restored or recreated.
pub fn on_level_change() {
    let mut inner = state();
    if !inner.enabled {
        return;
    }
    if inner.hidden != 0 {
        match native::host_condition(inner.hidden) {
            native::HostCondition::Minimized => return,
            native::HostCondition::Visible => {
                native::hide(inner.hidden);
                return;
            }
            native::HostCondition::Gone => inner.hidden = 0,
        }
    }
    let was_waiting = inner.waiting_since.is_some();
    let failure = inner.try_hide();
    if let Some(status) = failure {
        inner.emit(&status);
    } else if was_waiting && inner.hidden != 0 {
        let status = inner.status();
        inner.emit(&status);
    }
}

/// Explorer restarted (TaskbarCreated): the old host is gone and a new one is visible.
pub fn reapply() {
    let mut inner = state();
    if !inner.enabled {
        return;
    }
    inner.hidden = 0;
    if let Some(status) = inner.try_hide() {
        inner.emit(&status);
    }
}

/// Gives the flyout back without changing the saved preference.
pub fn shutdown() {
    state().restore();
}

#[tauri::command]
pub fn get_hide_system_flyout() -> FlyoutStatus {
    status()
}

#[tauri::command]
pub async fn set_hide_system_flyout(app: AppHandle, enabled: bool) -> Result<FlyoutStatus, String> {
    // Window lookups touch another process; keep them off the async runtime's workers.
    tauri::async_runtime::spawn_blocking(move || set_enabled(&app, enabled))
        .await
        .map_err(|_| "Could not update the flyout setting".to_string())?
}

#[cfg(windows)]
mod native {
    use super::{host_classes, pick_candidate, Candidate, Pick};
    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::{HWND, RECT},
            Graphics::Gdi::{
                GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
            },
            System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ},
            UI::WindowsAndMessaging::{
                FindWindowExW, GetShellWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic,
                IsWindow, ShowWindowAsync, SW_HIDE, SW_MINIMIZE, SW_RESTORE,
            },
        },
    };

    pub const SUPPORTED: bool = true;

    pub enum HostCondition {
        Minimized,
        Visible,
        Gone,
    }

    /// The real build number. `GetVersionEx` is manifest-dependent, the registry is not.
    fn build_number() -> Option<u32> {
        let mut buffer = [0u16; 32];
        let mut size = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
                w!("CurrentBuildNumber"),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status.is_err() {
            return None;
        }
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..len]).trim().parse().ok()
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn process_of(hwnd: HWND) -> u32 {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        pid
    }

    fn covers_monitor(hwnd: HWND) -> bool {
        unsafe {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_err() {
                return true;
            }
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return true;
            }
            let screen = info.rcMonitor;
            rect.right - rect.left >= screen.right - screen.left
                && rect.bottom - rect.top >= screen.bottom - screen.top
        }
    }

    pub fn find_host() -> Pick<isize> {
        let Some(build) = build_number() else {
            return Pick::NotFound;
        };
        let classes = host_classes(build);
        let shell = unsafe { GetShellWindow() };
        if shell.0.is_null() {
            return Pick::NotFound;
        }
        let shell_pid = process_of(shell);
        let outer = wide(classes.outer_class);
        let inner = wide(classes.inner_class);
        let inner_title = wide(classes.inner_title);
        let empty = wide("");
        let mut windows = Vec::new();
        let mut candidates = Vec::new();
        let mut after = HWND::default();
        // Bounded: the shell has a handful of these, never hundreds.
        for _ in 0..64 {
            let Ok(found) = (unsafe {
                FindWindowExW(
                    HWND::default(),
                    after,
                    PCWSTR(outer.as_ptr()),
                    PCWSTR(empty.as_ptr()),
                )
            }) else {
                break;
            };
            if found.0.is_null() {
                break;
            }
            after = found;
            let child = unsafe {
                FindWindowExW(
                    found,
                    HWND::default(),
                    PCWSTR(inner.as_ptr()),
                    PCWSTR(inner_title.as_ptr()),
                )
            };
            if !child.is_ok_and(|child| !child.0.is_null()) {
                continue;
            }
            windows.push(found);
            candidates.push(Candidate {
                shell_owned: process_of(found) == shell_pid,
                covers_monitor: covers_monitor(found),
            });
        }
        match pick_candidate(&candidates) {
            Pick::Found(index) => Pick::Found(windows[index].0 as isize),
            Pick::NotFound => Pick::NotFound,
            Pick::Ambiguous => Pick::Ambiguous,
        }
    }

    pub fn hide(raw: isize) {
        let hwnd = HWND(raw as _);
        unsafe {
            let _ = ShowWindowAsync(hwnd, SW_MINIMIZE);
        }
    }

    /// Restore, then hide: a restored host that stayed shown would be an invisible
    /// window blocking clicks. Windows shows it again on the next flyout.
    pub fn restore(raw: isize) {
        let hwnd = HWND(raw as _);
        unsafe {
            if IsWindow(hwnd).as_bool() {
                let _ = ShowWindowAsync(hwnd, SW_RESTORE);
                let _ = ShowWindowAsync(hwnd, SW_HIDE);
            }
        }
    }

    pub fn host_condition(raw: isize) -> HostCondition {
        let hwnd = HWND(raw as _);
        unsafe {
            if !IsWindow(hwnd).as_bool() {
                HostCondition::Gone
            } else if IsIconic(hwnd).as_bool() {
                HostCondition::Minimized
            } else {
                HostCondition::Visible
            }
        }
    }
}

#[cfg(not(windows))]
mod native {
    use super::Pick;

    pub const SUPPORTED: bool = false;

    pub enum HostCondition {
        Minimized,
        Visible,
        Gone,
    }

    pub fn find_host() -> Pick<isize> {
        Pick::NotFound
    }
    pub fn hide(_: isize) {}
    pub fn restore(_: isize) {}
    pub fn host_condition(_: isize) -> HostCondition {
        HostCondition::Gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn candidate(shell_owned: bool, covers_monitor: bool) -> Candidate {
        Candidate {
            shell_owned,
            covers_monitor,
        }
    }

    #[test]
    fn classes_follow_the_windows_version() {
        assert_eq!(host_classes(19045).outer_class, "NativeHWNDHost");
        assert_eq!(host_classes(19045).inner_class, "DirectUIHWND");
        assert_eq!(
            host_classes(22000).outer_class,
            "XamlExplorerHostIslandWindow"
        );
        assert_eq!(host_classes(26100).inner_title, "DesktopWindowXamlSource");
    }

    #[test]
    fn picks_the_only_small_shell_window() {
        // Windows 11 observed: the flyout plus a screen-sized XAML host.
        let found = [candidate(true, true), candidate(true, false)];
        assert_eq!(pick_candidate(&found), Pick::Found(1));
        assert_eq!(pick_candidate(&[]), Pick::NotFound);
        assert_eq!(pick_candidate(&[candidate(false, false)]), Pick::NotFound);
        assert_eq!(
            pick_candidate(&[candidate(true, false), candidate(true, false)]),
            Pick::Ambiguous
        );
    }

    #[test]
    fn gives_up_only_after_retries_over_time() {
        assert!(!should_give_up(0, GIVE_UP_AFTER * 2));
        assert!(!should_give_up(GIVE_UP_ATTEMPTS, GIVE_UP_AFTER / 2));
        assert!(should_give_up(GIVE_UP_ATTEMPTS, GIVE_UP_AFTER));
    }

    /// Read-only probe of the live shell (nothing is hidden). Run on a target build with
    /// `cargo test osd_flyout -- --ignored --nocapture` after one volume change.
    #[cfg(windows)]
    #[test]
    #[ignore = "inspects the live Windows shell"]
    fn probe_live_flyout_host() {
        let pick = native::find_host();
        println!("flyout host: {pick:?}");
        assert!(matches!(pick, Pick::Found(_)));
    }

    #[test]
    fn status_reports_enabled_only_while_active() {
        assert!(FlyoutStatus::new(FlyoutState::Hidden, None).enabled);
        assert!(FlyoutStatus::new(FlyoutState::Waiting, None).enabled);
        assert!(!FlyoutStatus::new(FlyoutState::Unavailable, Some("x")).enabled);
        let json = serde_json::to_value(FlyoutStatus::new(FlyoutState::Waiting, None)).unwrap();
        assert_eq!(json["state"], "waiting");
    }
}
