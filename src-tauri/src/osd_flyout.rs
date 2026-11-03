//! Opt-in hiding of the Windows volume/brightness flyout ("OSD").
//!
//! UNDOCUMENTED. Windows has no API to suppress its own flyout. This module uses the
//! technique community tools such as HideVolumeOSD rely on: find the shell window that
//! hosts the flyout and minimize it. A minimized host never paints, so the flyout stays
//! invisible until the window is restored. Everything here is a best-effort guess about
//! Explorer internals and can break with any Windows update:
//!
//! - Windows 10 (builds below 22000): a top-level `NativeHWNDHost` (empty title) with a
//!   `DirectUIHWND` child, drawn at the top left of the screen.
//! - Windows 11 (build 22000 and later): a top-level `XamlExplorerHostIslandWindow`
//!   (empty title) with a `Windows.UI.Composition.DesktopWindowContentBridge` child
//!   titled `DesktopWindowXamlSource`, drawn at the bottom centre. Explorer uses the
//!   same pair for other XAML surfaces, some of them screen-sized, so a candidate must
//!   also look like the flyout (`flyout_like`), and anything still ambiguous counts as
//!   "not found".
//!
//! The same host draws the brightness and media-key flyouts, so those are hidden too.
//! Safety rules:
//! - Off by default; only applied while Tuck runs.
//! - Explorer creates the host on the first flyout of a session, so nothing is looked
//!   up before a hardware key changed the level (`hardware_originated`); at autostart
//!   another shell window could otherwise pass for it. Tuck's own changes, the Quick
//!   Settings slider and apps changing the volume show no flyout, so they neither
//!   trigger a lookup nor count towards giving up.
//! - The stored window is re-checked (class pair and shell process) before every
//!   `ShowWindowAsync`, so a recycled handle is never touched.
//! - Restored on disable, on exit and while the island is hidden for a fullscreen app
//!   (so fullscreen apps keep their volume feedback). A marker file records that
//!   something may be minimized; it is only removed once a restore really ran, so a
//!   crash (release builds abort on panic) is repaired on the next start.
//! - If the window cannot be identified after changes that should have shown it, the
//!   setting turns itself off and says so until the user changes it again.
//! - Explorer restarts create a fresh host; `reapply` forgets the old one.
//! - Only `ShowWindowAsync` is used on the foreign window, so a hung Explorer can
//!   never block Tuck.

use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Service preference name (stored as `hide-system-flyout.json`).
pub const PREFERENCE: &str = "hide-system-flyout";
const MARKER: &str = "flyout-hidden.marker";
/// Tuck gives up only after this many failed lookups spread over at least
/// `GIVE_UP_AFTER`, each following a volume key press that should have shown the flyout.
pub(crate) const GIVE_UP_ATTEMPTS: u32 = 3;
pub(crate) const GIVE_UP_AFTER: Duration = Duration::from_secs(60);
/// Level changes this soon after Tuck's own wheel or mute request are Tuck's.
pub(crate) const SELF_CHANGE_WINDOW: Duration = Duration::from_millis(750);
/// Brightness keys have no key code, so a brightness change counts as hardware-originated
/// when the user touched an input device this recently.
pub(crate) const FLYOUT_INPUT_WINDOW: Duration = Duration::from_secs(2);
/// The lookup waits this long after a change, so Explorer has created the host.
pub(crate) const LOOKUP_DELAY: Duration = Duration::from_millis(400);
/// First build number of Windows 11.
pub(crate) const WINDOWS_11_BUILD: u32 = 22000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FlyoutState {
    /// The setting is off; the Windows flyout behaves normally.
    Off,
    /// The flyout host was found and is minimized.
    Hidden,
    /// Enabled, but the flyout is not hidden yet (or paused for a fullscreen app).
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

/// Where the flyout sits on its monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlyoutPlacement {
    /// Windows 11: above the taskbar, horizontally centred.
    BottomCentre,
    /// Windows 10: the top left corner.
    TopLeft,
}

/// Window class and title pairs that identify the flyout host on a given build.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct HostClasses {
    pub outer_class: &'static str,
    pub inner_class: &'static str,
    pub inner_title: &'static str,
    pub placement: FlyoutPlacement,
}

pub(crate) fn host_classes(build: u32) -> HostClasses {
    if build >= WINDOWS_11_BUILD {
        HostClasses {
            outer_class: "XamlExplorerHostIslandWindow",
            inner_class: "Windows.UI.Composition.DesktopWindowContentBridge",
            inner_title: "DesktopWindowXamlSource",
            placement: FlyoutPlacement::BottomCentre,
        }
    } else {
        HostClasses {
            outer_class: "NativeHWNDHost",
            inner_class: "DirectUIHWND",
            inner_title: "",
            placement: FlyoutPlacement::TopLeft,
        }
    }
}

/// Screen rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Bounds {
    fn width(self) -> i64 {
        i64::from(self.right) - i64::from(self.left)
    }
    fn height(self) -> i64 {
        i64::from(self.bottom) - i64::from(self.top)
    }
}

/// Whether a window has the flyout's shape: on its monitor, under a third of the
/// screen in each direction, and where the flyout is drawn on this Windows version.
/// Observed on build 26300 at 1920x1080: 169x61 at (875, 971).
pub(crate) fn flyout_like(placement: FlyoutPlacement, window: Bounds, monitor: Bounds) -> bool {
    let (width, height) = (window.width(), window.height());
    let (screen_width, screen_height) = (monitor.width(), monitor.height());
    if width <= 0 || height <= 0 || screen_width <= 0 || screen_height <= 0 {
        return false;
    }
    let inside = window.left >= monitor.left
        && window.top >= monitor.top
        && window.right <= monitor.right
        && window.bottom <= monitor.bottom;
    if !inside || width * 3 > screen_width || height * 3 > screen_height {
        return false;
    }
    // Doubled centres keep the arithmetic in integers.
    let centre_x = i64::from(window.left) + i64::from(window.right);
    let centre_y = i64::from(window.top) + i64::from(window.bottom);
    let screen_x = i64::from(monitor.left) + i64::from(monitor.right);
    let screen_y = i64::from(monitor.top) + i64::from(monitor.bottom);
    match placement {
        // Bottom half, centre within 5% of the screen width of the middle.
        FlyoutPlacement::BottomCentre => {
            centre_y >= screen_y && (centre_x - screen_x).abs() * 10 <= screen_width
        }
        FlyoutPlacement::TopLeft => centre_y < screen_y && centre_x < screen_x,
    }
}

/// A top-level window whose class pair matched.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    /// Owned by the shell (the process of `GetShellWindow`).
    pub shell_owned: bool,
    /// The window's rectangle, or its restored rectangle while minimized.
    pub bounds: Bounds,
    /// The monitor the window is on.
    pub monitor: Bounds,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pick<T = usize> {
    Found(T),
    NotFound,
    Ambiguous,
}

pub(crate) fn pick_candidate(placement: FlyoutPlacement, candidates: &[Candidate]) -> Pick {
    let mut matches = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate.shell_owned && flyout_like(placement, candidate.bounds, candidate.monitor)
        })
        .map(|(index, _)| index);
    match (matches.next(), matches.next()) {
        (None, _) => Pick::NotFound,
        (Some(index), None) => Pick::Found(index),
        (Some(_), Some(_)) => Pick::Ambiguous,
    }
}

/// Whether a level change at `now` came from Tuck's own request at `last_request`.
pub(crate) fn self_initiated(last_request: Option<Instant>, now: Instant) -> bool {
    last_request
        .and_then(|at| now.checked_duration_since(at))
        .is_some_and(|elapsed| elapsed <= SELF_CHANGE_WINDOW)
}

/// What changed, for `on_level_change`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelChange {
    Volume,
    Brightness,
}

/// Whether a level change came from a hardware key, so Windows showed its flyout and
/// the host exists. A volume change needs a volume key seen down. Brightness keys have
/// no key code, so recent input stands in; unknown input age counts.
pub(crate) fn hardware_originated(
    change: LevelChange,
    own_request: bool,
    volume_key: bool,
    input_age: Option<Duration>,
) -> bool {
    !own_request
        && match change {
            LevelChange::Volume => volume_key,
            LevelChange::Brightness => input_age.is_none_or(|age| age <= FLYOUT_INPUT_WINDOW),
        }
}

/// Give-up accounting for one lookup. Returns the message to give up with. Only a lookup
/// after a volume key counts (`counts`): a brightness change right after input may be the
/// Quick Settings slider, which shows no flyout. A found host starts the count over.
pub(crate) fn account_lookup(
    misses: &mut Misses,
    pick: &Pick<()>,
    counts: bool,
    now: Instant,
) -> Option<&'static str> {
    let message = match pick {
        Pick::Found(()) => {
            misses.reset();
            return None;
        }
        Pick::NotFound => NOT_FOUND,
        Pick::Ambiguous => AMBIGUOUS,
    };
    (counts && misses.record(now)).then_some(message)
}

/// Failed lookups since the setting was last enabled or the host was last found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Misses {
    count: u32,
    first: Option<Instant>,
}

impl Misses {
    pub(crate) const fn new() -> Self {
        Self {
            count: 0,
            first: None,
        }
    }

    /// Records a failed lookup. True when the setting should turn itself off.
    pub(crate) fn record(&mut self, now: Instant) -> bool {
        let first = *self.first.get_or_insert(now);
        self.count += 1;
        self.count >= GIVE_UP_ATTEMPTS
            && now.checked_duration_since(first).unwrap_or_default() >= GIVE_UP_AFTER
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::new();
    }
}

const NOT_FOUND: &str = "Turned off: Windows showed its volume flyout several times, but Tuck couldn't find it on this version of Windows";
const AMBIGUOUS: &str =
    "Turned off: Tuck couldn't tell the Windows volume flyout apart from other shell windows";
const WAITING: &str = "Takes effect after the next volume or brightness key press";
const PAUSED: &str = "Paused while a fullscreen app hides the island";

struct Inner {
    app: Option<AppHandle>,
    enabled: bool,
    /// Set on exit: nothing may minimize the host again.
    stopping: bool,
    /// The island is hidden for a fullscreen app; the flyout is given back meanwhile.
    fullscreen: bool,
    /// Raw HWND of the minimized host, 0 when not hidden.
    hidden: isize,
    /// A hardware key changed a level since Explorer started, so its host exists and a
    /// lookup is meaningful.
    flyout_seen: bool,
    /// When Tuck last changed the volume itself.
    last_request: Option<Instant>,
    /// A lookup due at this time, after a hardware-originated change.
    lookup_at: Option<Instant>,
    /// The due lookup follows a volume key, so a failure counts towards giving up.
    lookup_counts: bool,
    misses: Misses,
    /// A restore could not run (Explorer not up yet); retried on later events.
    repair_pending: bool,
    /// Why the setting last turned itself off; shown until the user changes it.
    failure: Option<&'static str>,
}

static STATE: Mutex<Inner> = Mutex::new(Inner {
    app: None,
    enabled: false,
    stopping: false,
    fullscreen: false,
    hidden: 0,
    flyout_seen: false,
    last_request: None,
    lookup_at: None,
    lookup_counts: false,
    misses: Misses::new(),
    repair_pending: false,
    failure: None,
});

fn state() -> std::sync::MutexGuard<'static, Inner> {
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Inner {
    fn status(&self) -> FlyoutStatus {
        if !self.enabled {
            match self.failure {
                Some(message) => FlyoutStatus::new(FlyoutState::Unavailable, Some(message)),
                None => FlyoutStatus::new(FlyoutState::Off, None),
            }
        } else if self.hidden != 0 {
            FlyoutStatus::new(FlyoutState::Hidden, None)
        } else if self.fullscreen {
            FlyoutStatus::new(FlyoutState::Waiting, Some(PAUSED))
        } else {
            FlyoutStatus::new(FlyoutState::Waiting, Some(WAITING))
        }
    }

    fn emit_status(&self) {
        if let Some(app) = &self.app {
            let _ = app.emit("system-flyout", self.status());
        }
    }

    /// Whether the host should be minimized right now.
    fn active(&self) -> bool {
        self.enabled && !self.stopping && !self.fullscreen
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

    /// Looks the host up and minimizes it. Returns false when it was not identified.
    fn lookup_and_hide(&mut self) -> Pick<()> {
        match native::find_host() {
            Pick::Found(hwnd) => {
                // Mark first: a crash between the two calls must still be repaired.
                self.marker(true);
                native::hide(hwnd);
                self.hidden = hwnd;
                self.misses.reset();
                Pick::Found(())
            }
            Pick::NotFound => Pick::NotFound,
            Pick::Ambiguous => Pick::Ambiguous,
        }
    }

    /// A lookup that follows a hardware-originated change.
    fn scheduled_lookup(&mut self, counts: bool, now: Instant) {
        let pick = self.lookup_and_hide();
        if pick == Pick::Found(()) {
            self.emit_status();
        }
        if let Some(message) = account_lookup(&mut self.misses, &pick, counts, now) {
            self.give_up(message);
        }
    }

    fn give_up(&mut self, message: &'static str) {
        self.restore();
        self.enabled = false;
        self.failure = Some(message);
        if let Some(app) = &self.app {
            if let Err(error) = crate::service_preferences::write(app, PREFERENCE, false) {
                eprintln!("Flyout preference: {error}");
            }
        }
        self.emit_status();
    }

    /// Gives the flyout back. The marker stays until a restore really ran: either the
    /// stored host was restored, or every minimized host was swept after a crash.
    fn restore(&mut self) {
        let mut done = false;
        if self.hidden != 0 {
            done = native::restore(self.hidden);
            self.hidden = 0;
        }
        self.lookup_at = None;
        self.lookup_counts = false;
        self.misses.reset();
        // Without a confirmed restore, sweep every minimized host the marker may mean.
        let present = self.marker_present();
        let repaired = done || !present || native::restore_leftovers().is_some();
        if repaired && present {
            self.marker(false);
        }
        self.repair_pending = !repaired;
    }

    /// Retries a restore that could not run while nothing should stay hidden.
    fn retry_repair(&mut self) {
        if self.repair_pending && !self.active() {
            self.restore();
        }
    }
}

/// Reads the preference and repairs a flyout left hidden by a crash. Nothing is looked
/// up yet: the host only exists after the first flyout of the session.
pub fn start(app: AppHandle) {
    let enabled = crate::service_preferences::read(&app, PREFERENCE);
    let mut inner = state();
    inner.app = Some(app);
    inner.enabled = enabled;
    if !enabled && inner.marker_present() {
        // A previous run hid the flyout and never restored it.
        inner.restore();
    }
    // While enabled a leftover minimized host simply stays hidden; the first lookup
    // adopts it (its restored rectangle still looks like the flyout).
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
    // The returned guard is not needed past the write.
    drop(crate::service_preferences::write(app, PREFERENCE, enabled)?);
    inner.enabled = enabled;
    // The user decided again; the last failure no longer applies.
    inner.failure = None;
    inner.misses.reset();
    inner.lookup_at = None;
    inner.lookup_counts = false;
    if enabled {
        // Only look now if a flyout already appeared this session; a miss here does
        // not count, the next key press retries.
        if inner.active() && inner.hidden == 0 && inner.flyout_seen {
            let _ = inner.lookup_and_hide();
        }
    } else {
        inner.restore();
    }
    let status = inner.status();
    inner.emit_status();
    Ok(status)
}

/// Tuck is about to change the volume itself (island wheel or middle click); the
/// resulting level change cannot have shown the Windows flyout.
pub fn note_self_change() {
    state().last_request = Some(Instant::now());
}

/// Called on every volume or brightness change.
pub fn on_level_change(change: LevelChange) {
    let mut inner = state();
    if inner.stopping {
        return;
    }
    inner.retry_repair();
    let now = Instant::now();
    let own = self_initiated(inner.last_request, now);
    let volume_key = crate::desktop_signals::volume_key_active();
    let input_age = crate::desktop_signals::user_input_age();
    if !hardware_originated(change, own, volume_key, input_age) {
        return;
    }
    inner.flyout_seen = true;
    if !inner.active() {
        return;
    }
    if inner.hidden != 0 {
        match native::host_condition(inner.hidden) {
            native::HostCondition::Minimized => return,
            native::HostCondition::Visible => {
                // Explorer restored it; minimize it again (already re-validated).
                native::hide(inner.hidden);
                return;
            }
            native::HostCondition::Gone => inner.hidden = 0,
        }
    }
    // Explorer may still be creating the host; look once things settle.
    if inner.lookup_at.is_none() {
        inner.lookup_at = Some(now + LOOKUP_DELAY);
    }
    inner.lookup_counts |= change == LevelChange::Volume;
}

/// Runs a due lookup. Called from the desktop-signals loop, at most every 250ms.
pub fn tick() {
    let mut inner = state();
    let Some(due) = inner.lookup_at else {
        return;
    };
    let now = Instant::now();
    if now < due {
        return;
    }
    inner.lookup_at = None;
    let counts = std::mem::take(&mut inner.lookup_counts);
    if inner.active() && inner.hidden == 0 {
        inner.scheduled_lookup(counts, now);
    }
}

/// The island was hidden for (or came back from) a fullscreen app. While hidden, the
/// Windows flyout is the only volume feedback, so it is given back meanwhile.
pub fn set_fullscreen(active: bool) {
    let mut inner = state();
    if inner.fullscreen == active || inner.stopping {
        inner.fullscreen = active;
        return;
    }
    inner.fullscreen = active;
    if !inner.enabled {
        return;
    }
    if active {
        inner.restore();
    } else if inner.flyout_seen {
        let _ = inner.lookup_and_hide();
    }
    inner.emit_status();
}

/// Explorer restarted (TaskbarCreated): a stored host that is gone belongs to the old
/// Explorer, and the new one creates its host on its first flyout.
pub fn reapply() {
    let mut inner = state();
    if inner.stopping {
        return;
    }
    inner.retry_repair();
    if inner.hidden == 0 {
        // Possibly a new Explorer without a host yet; the next key press proves it.
        inner.flyout_seen = false;
        return;
    }
    match native::host_condition(inner.hidden) {
        native::HostCondition::Minimized => {}
        native::HostCondition::Visible => {
            if inner.active() {
                native::hide(inner.hidden);
            }
        }
        native::HostCondition::Gone => {
            inner.hidden = 0;
            inner.flyout_seen = false;
            inner.misses.reset();
            inner.emit_status();
        }
    }
}

/// Gives the flyout back without changing the saved preference. Queued level changes
/// arriving afterwards are ignored.
pub fn shutdown() {
    let mut inner = state();
    inner.stopping = true;
    inner.restore();
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
    use super::{host_classes, pick_candidate, Bounds, Candidate, HostClasses, Pick};
    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::{HWND, RECT},
            Graphics::Gdi::{GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONULL},
            System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ},
            UI::WindowsAndMessaging::{
                FindWindowExW, GetClassNameW, GetShellWindow, GetWindowPlacement, GetWindowRect,
                GetWindowThreadProcessId, IsIconic, IsWindow, ShowWindowAsync, SW_HIDE,
                SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, WINDOWPLACEMENT,
            },
        },
    };

    pub const SUPPORTED: bool = true;

    pub enum HostCondition {
        Minimized,
        Visible,
        /// Destroyed, or the handle now names a different window.
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

    fn classes() -> Option<HostClasses> {
        build_number().map(host_classes)
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

    fn shell_process() -> Option<u32> {
        let shell = unsafe { GetShellWindow() };
        if shell.0.is_null() {
            return None;
        }
        Some(process_of(shell)).filter(|pid| *pid != 0)
    }

    fn has_inner(hwnd: HWND, classes: &HostClasses) -> bool {
        let inner = wide(classes.inner_class);
        let title = wide(classes.inner_title);
        unsafe {
            FindWindowExW(
                hwnd,
                HWND::default(),
                PCWSTR(inner.as_ptr()),
                PCWSTR(title.as_ptr()),
            )
        }
        .is_ok_and(|child| !child.0.is_null())
    }

    /// Still the flyout host: a live window with the class pair, owned by the shell.
    fn is_host(hwnd: HWND, classes: &HostClasses) -> bool {
        if !unsafe { IsWindow(hwnd) }.as_bool() {
            return false;
        }
        let mut name = [0u16; 128];
        let len = unsafe { GetClassNameW(hwnd, &mut name) };
        let len = usize::try_from(len).unwrap_or(0).min(name.len());
        String::from_utf16_lossy(&name[..len]) == classes.outer_class
            && has_inner(hwnd, classes)
            && shell_process() == Some(process_of(hwnd))
    }

    /// The window's rectangle; for a minimized window, the one it restores to.
    fn bounds(hwnd: HWND) -> Option<Bounds> {
        let rect = unsafe {
            if IsIconic(hwnd).as_bool() {
                let mut placement = WINDOWPLACEMENT {
                    length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                    ..Default::default()
                };
                GetWindowPlacement(hwnd, &mut placement).ok()?;
                // Screen coordinates: the host is a tool window.
                placement.rcNormalPosition
            } else {
                let mut rect = RECT::default();
                GetWindowRect(hwnd, &mut rect).ok()?;
                rect
            }
        };
        Some(Bounds {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
    }

    fn monitor_of(bounds: Bounds) -> Option<Bounds> {
        let rect = RECT {
            left: bounds.left,
            top: bounds.top,
            right: bounds.right,
            bottom: bounds.bottom,
        };
        unsafe {
            let monitor = MonitorFromRect(&rect, MONITOR_DEFAULTTONULL);
            if monitor.is_invalid() {
                return None;
            }
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return None;
            }
            let screen = info.rcMonitor;
            Some(Bounds {
                left: screen.left,
                top: screen.top,
                right: screen.right,
                bottom: screen.bottom,
            })
        }
    }

    /// Shell-owned top-level windows with the class pair.
    fn hosts(classes: &HostClasses) -> Option<Vec<HWND>> {
        let shell_pid = shell_process()?;
        let outer = wide(classes.outer_class);
        let empty = wide("");
        let mut windows = Vec::new();
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
            if process_of(found) == shell_pid && has_inner(found, classes) {
                windows.push(found);
            }
        }
        Some(windows)
    }

    pub fn find_host() -> Pick<isize> {
        let Some(classes) = classes() else {
            return Pick::NotFound;
        };
        let Some(hosts) = hosts(&classes) else {
            return Pick::NotFound;
        };
        let mut windows = Vec::new();
        let mut candidates = Vec::new();
        for hwnd in hosts {
            let Some(bounds) = bounds(hwnd) else {
                continue;
            };
            let Some(monitor) = monitor_of(bounds) else {
                continue;
            };
            windows.push(hwnd);
            candidates.push(Candidate {
                shell_owned: true,
                bounds,
                monitor,
            });
        }
        match pick_candidate(classes.placement, &candidates) {
            Pick::Found(index) => Pick::Found(windows[index].0 as isize),
            Pick::NotFound => Pick::NotFound,
            Pick::Ambiguous => Pick::Ambiguous,
        }
    }

    /// Minimizes the host without activating anything, after re-validating it.
    pub fn hide(raw: isize) {
        let hwnd = HWND(raw as _);
        let Some(classes) = classes() else {
            return;
        };
        if is_host(hwnd, &classes) && !unsafe { IsIconic(hwnd) }.as_bool() {
            unsafe {
                let _ = ShowWindowAsync(hwnd, SW_SHOWMINNOACTIVE);
            }
        }
    }

    /// Un-minimize without activation, then hide: a restored host that stayed shown
    /// would be an invisible window blocking clicks. Windows shows it again on the next
    /// flyout. Only for a window that `is_host` accepted.
    fn give_back(hwnd: HWND) {
        unsafe {
            let _ = ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE);
            let _ = ShowWindowAsync(hwnd, SW_HIDE);
        }
    }

    /// Restores the stored host. True when nothing Tuck minimized can remain: it was
    /// restored, or it no longer exists.
    pub fn restore(raw: isize) -> bool {
        let hwnd = HWND(raw as _);
        if !unsafe { IsWindow(hwnd) }.as_bool() {
            return true;
        }
        let Some(classes) = classes() else {
            return false;
        };
        if !is_host(hwnd, &classes) {
            // The handle was recycled, so the original window is gone too. Unless the
            // shell process could not be read; then let the sweep decide.
            return shell_process().is_some();
        }
        if unsafe { IsIconic(hwnd) }.as_bool() {
            give_back(hwnd);
        }
        true
    }

    /// Crash repair: restores every minimized shell-owned window with the class pair.
    /// `None` when the shell could not be inspected (Explorer not running yet).
    pub fn restore_leftovers() -> Option<usize> {
        let classes = classes()?;
        let mut restored = 0;
        for hwnd in hosts(&classes)? {
            if unsafe { IsIconic(hwnd) }.as_bool() {
                give_back(hwnd);
                restored += 1;
            }
        }
        Some(restored)
    }

    pub fn host_condition(raw: isize) -> HostCondition {
        let hwnd = HWND(raw as _);
        let Some(classes) = classes() else {
            return HostCondition::Gone;
        };
        if !is_host(hwnd, &classes) {
            HostCondition::Gone
        } else if unsafe { IsIconic(hwnd) }.as_bool() {
            HostCondition::Minimized
        } else {
            HostCondition::Visible
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
    pub fn restore(_: isize) -> bool {
        true
    }
    pub fn restore_leftovers() -> Option<usize> {
        Some(0)
    }
    pub fn host_condition(_: isize) -> HostCondition {
        HostCondition::Gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Bounds = Bounds {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };
    /// The Windows 11 flyout host observed on build 26300.
    const FLYOUT: Bounds = Bounds {
        left: 875,
        top: 971,
        right: 1044,
        bottom: 1032,
    };

    const fn candidate(shell_owned: bool, bounds: Bounds) -> Candidate {
        Candidate {
            shell_owned,
            bounds,
            monitor: SCREEN,
        }
    }

    const fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Bounds {
        Bounds {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn classes_follow_the_windows_version() {
        assert_eq!(host_classes(19045).outer_class, "NativeHWNDHost");
        assert_eq!(host_classes(19045).inner_class, "DirectUIHWND");
        assert_eq!(host_classes(19045).placement, FlyoutPlacement::TopLeft);
        assert_eq!(
            host_classes(22000).outer_class,
            "XamlExplorerHostIslandWindow"
        );
        assert_eq!(host_classes(26100).inner_title, "DesktopWindowXamlSource");
        assert_eq!(host_classes(26100).placement, FlyoutPlacement::BottomCentre);
    }

    #[test]
    fn geometry_accepts_only_flyout_shaped_windows() {
        let bottom = FlyoutPlacement::BottomCentre;
        assert!(flyout_like(bottom, FLYOUT, SCREEN));
        // Screen-sized XAML host
        assert!(!flyout_like(bottom, SCREEN, SCREEN));
        // Small but in the top half, or off centre (a corner toast host)
        assert!(!flyout_like(bottom, rect(875, 40, 1044, 101), SCREEN));
        assert!(!flyout_like(bottom, rect(1500, 971, 1669, 1032), SCREEN));
        // A third of the screen or more in either direction
        assert!(!flyout_like(bottom, rect(640, 600, 1280, 1000), SCREEN));
        assert!(!flyout_like(bottom, rect(900, 500, 1020, 1080), SCREEN));
        // Off the monitor, empty, or a minimized window's parking spot
        assert!(!flyout_like(bottom, rect(875, 1071, 1044, 1132), SCREEN));
        assert!(!flyout_like(bottom, rect(900, 1000, 900, 1000), SCREEN));
        assert!(!flyout_like(
            bottom,
            rect(-32000, -32000, -31840, -31972),
            SCREEN
        ));
        // A second monitor to the right, offset vertically
        let right = rect(1920, -200, 4480, 1240);
        assert!(flyout_like(bottom, rect(3115, 1100, 3285, 1160), right));
        // Windows 10 draws it at the top left
        let top_left = FlyoutPlacement::TopLeft;
        assert!(flyout_like(top_left, rect(50, 60, 115, 220), SCREEN));
        assert!(!flyout_like(top_left, FLYOUT, SCREEN));
    }

    #[test]
    fn picks_the_only_flyout_shaped_shell_window() {
        let bottom = FlyoutPlacement::BottomCentre;
        // Windows 11 observed: a screen-sized XAML host plus the flyout.
        let found = [candidate(true, SCREEN), candidate(true, FLYOUT)];
        assert_eq!(pick_candidate(bottom, &found), Pick::Found(1));
        assert_eq!(pick_candidate(bottom, &[]), Pick::NotFound);
        // Before the first flyout only other small shell hosts exist: never guessed.
        let toast = rect(1500, 900, 1900, 1030);
        assert_eq!(
            pick_candidate(bottom, &[candidate(true, SCREEN), candidate(true, toast)]),
            Pick::NotFound
        );
        assert_eq!(
            pick_candidate(bottom, &[candidate(false, FLYOUT)]),
            Pick::NotFound
        );
        assert_eq!(
            pick_candidate(bottom, &[candidate(true, FLYOUT), candidate(true, FLYOUT)]),
            Pick::Ambiguous
        );
    }

    #[test]
    fn own_changes_are_recognised_for_a_short_window() {
        let at = Instant::now();
        assert!(!self_initiated(None, at));
        assert!(self_initiated(Some(at), at));
        assert!(self_initiated(Some(at), at + SELF_CHANGE_WINDOW));
        assert!(!self_initiated(
            Some(at),
            at + SELF_CHANGE_WINDOW + Duration::from_millis(1)
        ));
        // A request stamped after the change (clock order) is not the change's cause
        assert!(!self_initiated(Some(at + Duration::from_secs(1)), at));
    }

    #[test]
    fn only_hardware_keys_count_as_flyout_changes() {
        use LevelChange::{Brightness, Volume};
        let recent = Some(Duration::from_millis(100));
        let stale = Some(Duration::from_secs(30));
        // A volume key: yes. The Quick Settings slider or an app: recent input, no key.
        assert!(hardware_originated(Volume, false, true, recent));
        assert!(!hardware_originated(Volume, false, false, recent));
        assert!(!hardware_originated(Volume, false, false, None));
        // Tuck's own wheel step, even if a key happens to be down
        assert!(!hardware_originated(Volume, true, true, recent));
        // Brightness keys have no key code: recent input stands in
        assert!(hardware_originated(Brightness, false, false, recent));
        assert!(hardware_originated(Brightness, false, false, None));
        assert!(hardware_originated(
            Brightness,
            false,
            false,
            Some(FLYOUT_INPUT_WINDOW)
        ));
        assert!(!hardware_originated(Brightness, false, false, stale));
        assert!(!hardware_originated(Brightness, true, false, recent));
    }

    #[test]
    fn only_volume_key_lookups_count_towards_giving_up() {
        let start = Instant::now();
        let later = |seconds| start + Duration::from_secs(seconds);
        let mut misses = Misses::new();
        // Lookups after brightness changes never give up, however many fail
        for seconds in 0..10 {
            let outcome = account_lookup(&mut misses, &Pick::NotFound, false, later(seconds * 60));
            assert_eq!(outcome, None);
        }
        // Volume keys do, once there are enough misses over enough time
        assert_eq!(
            account_lookup(&mut misses, &Pick::NotFound, true, start),
            None
        );
        assert_eq!(
            account_lookup(&mut misses, &Pick::Ambiguous, true, later(1)),
            None
        );
        assert_eq!(
            account_lookup(&mut misses, &Pick::NotFound, true, later(2)),
            None
        );
        // Finding the host starts over
        assert_eq!(
            account_lookup(&mut misses, &Pick::Found(()), true, later(3)),
            None
        );
        assert_eq!(misses, Misses::new());
        assert_eq!(
            account_lookup(&mut misses, &Pick::NotFound, true, start),
            None
        );
        assert_eq!(
            account_lookup(&mut misses, &Pick::NotFound, true, later(30)),
            None
        );
        assert_eq!(
            account_lookup(&mut misses, &Pick::Ambiguous, true, later(60)),
            Some(AMBIGUOUS)
        );
    }

    #[test]
    fn gives_up_only_after_misses_over_time() {
        let start = Instant::now();
        let mut misses = Misses::new();
        assert!(!misses.record(start));
        assert!(!misses.record(start + Duration::from_secs(1)));
        // Three misses, but within a minute
        assert!(!misses.record(start + Duration::from_secs(2)));
        assert!(misses.record(start + GIVE_UP_AFTER));
        // Spread out, but only two
        misses.reset();
        assert!(!misses.record(start));
        assert!(!misses.record(start + GIVE_UP_AFTER * 2));
        assert!(misses.record(start + GIVE_UP_AFTER * 3));
    }

    /// Read-only probe of the live shell (nothing is hidden). Run on a target build with
    /// `cargo test osd_flyout -- --ignored --nocapture` after one volume key press.
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

    #[test]
    fn a_failure_stays_visible_until_the_user_changes_the_setting() {
        let mut inner = Inner {
            app: None,
            enabled: false,
            stopping: false,
            fullscreen: false,
            hidden: 0,
            flyout_seen: true,
            last_request: None,
            lookup_at: None,
            lookup_counts: false,
            misses: Misses::new(),
            repair_pending: false,
            failure: Some(NOT_FOUND),
        };
        let status = inner.status();
        assert_eq!(status.state, FlyoutState::Unavailable);
        assert_eq!(status.message.as_deref(), Some(NOT_FOUND));
        assert!(!status.enabled);
        inner.failure = None;
        assert_eq!(inner.status().state, FlyoutState::Off);
        inner.enabled = true;
        inner.fullscreen = true;
        assert_eq!(inner.status().message.as_deref(), Some(PAUSED));
        assert!(!inner.active());
        inner.fullscreen = false;
        inner.stopping = true;
        assert!(!inner.active(), "Nothing is minimized again after exit");
    }
}
