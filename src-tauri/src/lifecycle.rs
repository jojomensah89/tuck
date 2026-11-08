//! One hidden top-level Win32 window owns lifecycle notifications for the island.
//! It is never shown; it exists because broadcasts such as suspend/resume, display and
//! setting changes and `TaskbarCreated` are not delivered to message-only windows.

use std::time::{Duration, Instant};
use tauri::AppHandle;

/// Quiet period after the last foreground move/resize before fullscreen is re-checked.
pub(crate) const LOCATION_QUIET: Duration = Duration::from_millis(200);
/// Upper bound on how long a continuous stream of moves can defer the check.
pub(crate) const LOCATION_MAX_WAIT: Duration = Duration::from_millis(1000);
/// Safety refresh interval, only armed while the island is hidden for fullscreen.
pub(crate) const FULLSCREEN_RECHECK: Duration = Duration::from_millis(1000);
/// Window in which a second resume notification is treated as a duplicate.
pub(crate) const RESUME_DEDUPE: Duration = Duration::from_secs(5);

/// What a debounce timer should do when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DebounceStep {
    /// The burst ended (or ran too long): run the deferred work now.
    Fire,
    /// Re-arm the timer for this long.
    Wait(Duration),
    /// Nothing pending; stop the timer.
    Idle,
}

/// Trailing-edge debounce with a maximum wait, driven by a single Win32 timer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Debouncer {
    quiet: Duration,
    max_wait: Duration,
    first: Option<Instant>,
    last: Option<Instant>,
}

impl Debouncer {
    pub(crate) const fn new(quiet: Duration, max_wait: Duration) -> Self {
        Self {
            quiet,
            max_wait,
            first: None,
            last: None,
        }
    }

    /// Records an event. Returns true when this starts a burst and a timer must be armed.
    pub(crate) fn event(&mut self, now: Instant) -> bool {
        let starts_burst = self.first.is_none();
        if starts_burst {
            self.first = Some(now);
        }
        self.last = Some(now);
        starts_burst
    }

    pub(crate) fn poll(&mut self, now: Instant) -> DebounceStep {
        let (Some(first), Some(last)) = (self.first, self.last) else {
            return DebounceStep::Idle;
        };
        let quiet_left = self
            .quiet
            .saturating_sub(now.saturating_duration_since(last));
        let max_left = self
            .max_wait
            .saturating_sub(now.saturating_duration_since(first));
        if quiet_left.is_zero() || max_left.is_zero() {
            self.first = None;
            self.last = None;
            DebounceStep::Fire
        } else {
            DebounceStep::Wait(quiet_left.min(max_left))
        }
    }
}

/// Suspend/resume can arrive both as a broadcast and through the suspend-resume
/// registration, and resume comes as both RESUMEAUTOMATIC and RESUMESUSPEND.
pub(crate) fn is_duplicate_resume(
    suspended: bool,
    last_resume: Option<Instant>,
    now: Instant,
) -> bool {
    !suspended && last_resume.is_some_and(|at| now.saturating_duration_since(at) < RESUME_DEDUPE)
}

fn update_clipboard_registration<E>(
    registered: &mut bool,
    enabled: bool,
    apply: impl FnOnce(bool) -> Result<(), E>,
) -> Result<bool, E> {
    if *registered == enabled {
        return Ok(false);
    }
    apply(enabled)?;
    *registered = enabled;
    Ok(true)
}

/// Foreground hook ownership deliberately excludes the clipboard subscription.
fn rebind_foreground<H: Copy + Default>(
    pid: u32,
    current: &mut u32,
    hook: &mut H,
    valid: impl Fn(H) -> bool,
    remove: impl FnOnce(H),
    add: impl FnOnce(u32) -> H,
) {
    if pid == *current && (pid == 0 || valid(*hook)) {
        return;
    }
    if valid(*hook) {
        remove(*hook);
    }
    *hook = H::default();
    *current = 0;
    if pid != 0 {
        *hook = add(pid);
        *current = pid;
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        panic::{catch_unwind, AssertUnwindSafe},
        sync::atomic::{AtomicBool, AtomicIsize, Ordering},
    };
    use tauri::{Emitter, Manager};
    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::{HANDLE, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, WPARAM},
            System::{
                LibraryLoader::GetModuleHandleW,
                Power::{
                    RegisterPowerSettingNotification, RegisterSuspendResumeNotification,
                    UnregisterPowerSettingNotification, UnregisterSuspendResumeNotification,
                    HPOWERNOTIFY, POWERBROADCAST_SETTING,
                },
                RemoteDesktop::{
                    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
                    NOTIFY_FOR_THIS_SESSION,
                },
                SystemServices::{
                    GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING,
                    GUID_CONSOLE_DISPLAY_STATE,
                },
                Threading::GetCurrentProcessId,
            },
            UI::{
                Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
                WindowsAndMessaging::{
                    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
                    GetForegroundWindow, GetMessageW, GetWindowLongPtrW, GetWindowThreadProcessId,
                    KillTimer, PostMessageW, PostQuitMessage, RegisterClassW,
                    RegisterWindowMessageW, SetTimer, SetWindowLongPtrW, TranslateMessage,
                    UnregisterClassW, CREATESTRUCTW, DEVICE_NOTIFY_WINDOW_HANDLE,
                    EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND, GWLP_USERDATA, HMENU,
                    MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
                    PBT_POWERSETTINGCHANGE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP,
                    WM_DISPLAYCHANGE, WM_NCCREATE, WM_POWERBROADCAST, WM_SETTINGCHANGE, WM_TIMER,
                    WM_WTSSESSION_CHANGE, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
                    WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
                },
            },
        },
    };

    const WM_TUCK_FOREGROUND: u32 = WM_APP + 41;
    const WM_TUCK_STOP: u32 = WM_APP + 42;
    const WM_TUCK_LOCATION: u32 = WM_APP + 43;
    const WM_TUCK_REFRESH: u32 = WM_APP + 44;
    const RESUME_TIMER: usize = 1;
    const LOCATION_TIMER: usize = 2;
    const FULLSCREEN_TIMER: usize = 3;
    const OBJID_WINDOW: i32 = 0;
    const CHILDID_SELF: i32 = 0;
    static EVENT_WINDOW: AtomicIsize = AtomicIsize::new(0);
    /// Foreground window whose moves/resizes are forwarded to the hub.
    static FOREGROUND: AtomicIsize = AtomicIsize::new(0);
    /// Coalesces location posts so a window drag does not flood the queue.
    static LOCATION_POSTED: AtomicBool = AtomicBool::new(false);

    fn timer_ms(duration: Duration) -> u32 {
        duration.as_millis().clamp(10, u32::MAX as u128) as u32
    }

    struct Context {
        app: AppHandle,
        hwnd: HWND,
        taskbar_created: u32,
        locked: bool,
        suspended: bool,
        display_off: bool,
        last_resume: Option<Instant>,
        location_hook: HWINEVENTHOOK,
        location_pid: u32,
        location_debounce: Debouncer,
        fullscreen_timer: bool,
        clipboard_listener: bool,
    }

    impl Context {
        fn update_clipboard_listener(&mut self) {
            use windows::Win32::System::DataExchange::{
                AddClipboardFormatListener, RemoveClipboardFormatListener,
            };
            let enabled = crate::clipboard_history::listener_enabled(&self.app);
            let hwnd = self.hwnd;
            let result = update_clipboard_registration(
                &mut self.clipboard_listener,
                enabled,
                |enabled| unsafe {
                    if enabled {
                        AddClipboardFormatListener(hwnd)
                    } else {
                        RemoveClipboardFormatListener(hwnd)
                    }
                },
            );
            match result {
                Ok(changed) => {
                    if changed && enabled {
                        crate::clipboard_history::clipboard_updated();
                    }
                }
                Err(error) => {
                    let _ = self.app.emit(
                        "clipboard_history_error",
                        format!("Clipboard listener: {error}"),
                    );
                }
            }
        }

        fn emit_power(&self) {
            if let Some(power) = crate::system::read_power() {
                let _ = self.app.emit("power", power);
            }
        }

        fn island_hidden(&self) -> bool {
            self.locked || self.suspended || self.display_off
        }

        fn hide_island(&self) {
            if let Some(win) = self.app.get_webview_window("main") {
                #[cfg(not(windows))]
                let _ = win.set_ignore_cursor_events(true);
                let _ = win.hide();
            }
        }

        /// Re-checks fullscreen first; `refresh_window_state` only re-raises TOPMOST when
        /// the island is not hidden for fullscreen. The 1s safety timer runs only while
        /// hidden, so exiting fullscreen without a move event cannot strand the island.
        fn refresh_fullscreen(&mut self) {
            let hidden = crate::system::refresh_window_state(&self.app);
            if hidden && !self.fullscreen_timer {
                unsafe {
                    SetTimer(
                        self.hwnd,
                        FULLSCREEN_TIMER,
                        timer_ms(FULLSCREEN_RECHECK),
                        None,
                    );
                }
                self.fullscreen_timer = true;
            } else if !hidden && self.fullscreen_timer {
                unsafe {
                    let _ = KillTimer(self.hwnd, FULLSCREEN_TIMER);
                }
                self.fullscreen_timer = false;
            }
        }

        fn restore_island(&mut self) {
            if self.island_hidden() {
                return;
            }
            if let Some(win) = self.app.get_webview_window("main") {
                let _ = crate::system::center_on_primary_monitor(&win);
                let _ = win.show();
            }
            self.refresh_fullscreen();
        }

        fn refresh_layout(&mut self) {
            if let Some(win) = self.app.get_webview_window("main") {
                let _ = crate::system::center_on_primary_monitor(&win);
            }
            if self.island_hidden() {
                return;
            }
            self.refresh_fullscreen();
        }

        /// Follows the foreground window with a location hook scoped to its process, so
        /// in-app fullscreen toggles (F11, video players, Alt+Enter) are noticed.
        fn watch_foreground(&mut self) {
            let foreground = unsafe { GetForegroundWindow() };
            FOREGROUND.store(foreground.0 as isize, Ordering::SeqCst);
            let mut pid = 0u32;
            if !foreground.0.is_null() {
                unsafe {
                    GetWindowThreadProcessId(foreground, Some(&mut pid));
                }
            }
            if pid == unsafe { GetCurrentProcessId() } {
                pid = 0;
            }
            rebind_foreground(
                pid,
                &mut self.location_pid,
                &mut self.location_hook,
                |hook| !hook.0.is_null(),
                |hook| {
                    if !unsafe { UnhookWinEvent(hook) }.as_bool() {
                        eprintln!("Location hook cleanup failed");
                    }
                },
                |pid| {
                    let hook = unsafe {
                        SetWinEventHook(
                            EVENT_OBJECT_LOCATIONCHANGE,
                            EVENT_OBJECT_LOCATIONCHANGE,
                            HMODULE::default(),
                            Some(location_callback),
                            pid,
                            0,
                            WINEVENT_OUTOFCONTEXT,
                        )
                    };
                    if hook.0.is_null() {
                        eprintln!("Location hook registration failed");
                    }
                    hook
                },
            );
        }

        fn unhook_location(&mut self) {
            rebind_foreground(
                0,
                &mut self.location_pid,
                &mut self.location_hook,
                |hook| !hook.0.is_null(),
                |hook| {
                    let _ = unsafe { UnhookWinEvent(hook) };
                },
                |_| HWINEVENTHOOK::default(),
            );
        }

        fn location_changed(&mut self) {
            LOCATION_POSTED.store(false, Ordering::SeqCst);
            if self.location_debounce.event(Instant::now()) {
                unsafe {
                    SetTimer(self.hwnd, LOCATION_TIMER, timer_ms(LOCATION_QUIET), None);
                }
            }
        }

        fn location_timer(&mut self) {
            match self.location_debounce.poll(Instant::now()) {
                DebounceStep::Fire => {
                    unsafe {
                        let _ = KillTimer(self.hwnd, LOCATION_TIMER);
                    }
                    if !self.island_hidden() {
                        self.refresh_fullscreen();
                    }
                }
                DebounceStep::Wait(delay) => unsafe {
                    SetTimer(self.hwnd, LOCATION_TIMER, timer_ms(delay), None);
                },
                DebounceStep::Idle => unsafe {
                    let _ = KillTimer(self.hwnd, LOCATION_TIMER);
                },
            }
        }

        fn suspend(&mut self) {
            if self.suspended {
                return;
            }
            self.suspended = true;
            crate::system::set_cursor_pause(crate::system::CURSOR_PAUSE_SUSPEND, true);
            self.hide_island();
            crate::media::suspend();
            crate::desktop_signals::suspend();
        }

        fn resume(&mut self) {
            let now = Instant::now();
            if is_duplicate_resume(self.suspended, self.last_resume, now) {
                return;
            }
            self.last_resume = Some(now);
            self.suspended = false;
            crate::system::set_cursor_pause(crate::system::CURSOR_PAUSE_SUSPEND, self.display_off);
            crate::media::resume();
            crate::desktop_signals::resume();
            self.emit_power();
            self.watch_foreground();
            self.restore_island();
        }

        fn lock(&mut self) {
            self.locked = true;
            crate::system::set_cursor_pause(crate::system::CURSOR_PAUSE_SESSION, true);
            self.hide_island();
        }

        fn unlock(&mut self) {
            self.locked = false;
            crate::system::set_cursor_pause(crate::system::CURSOR_PAUSE_SESSION, false);
            unsafe {
                SetTimer(self.hwnd, RESUME_TIMER, 150, None);
            }
        }

        fn power_setting(&mut self, setting: *const POWERBROADCAST_SETTING) {
            if setting.is_null() {
                return;
            }
            let setting = unsafe { &*setting };
            if setting.PowerSetting == GUID_CONSOLE_DISPLAY_STATE && setting.DataLength >= 4 {
                let state =
                    unsafe { std::ptr::read_unaligned(setting.Data.as_ptr().cast::<u32>()) };
                self.display_off = state == 0;
                crate::system::set_cursor_pause(
                    crate::system::CURSOR_PAUSE_SUSPEND,
                    self.suspended || self.display_off,
                );
                if self.display_off {
                    self.hide_island();
                } else {
                    self.restore_island();
                }
            }
            self.emit_power();
        }
    }

    impl Drop for Context {
        fn drop(&mut self) {
            if self.clipboard_listener {
                let _ = unsafe {
                    windows::Win32::System::DataExchange::RemoveClipboardFormatListener(self.hwnd)
                };
                self.clipboard_listener = false;
            }
            self.unhook_location();
        }
    }

    struct Registrations {
        hwnd: HWND,
        power: Vec<HPOWERNOTIFY>,
        suspend_resume: Option<HPOWERNOTIFY>,
        foreground: HWINEVENTHOOK,
        session_registered: bool,
        class_name: Vec<u16>,
        instance: HINSTANCE,
    }

    impl Drop for Registrations {
        fn drop(&mut self) {
            unsafe {
                for handle in self.power.drain(..) {
                    if let Err(error) = UnregisterPowerSettingNotification(handle) {
                        eprintln!("Power notification cleanup: {error}");
                    }
                }
                if let Some(handle) = self.suspend_resume.take() {
                    if let Err(error) = UnregisterSuspendResumeNotification(handle) {
                        eprintln!("Suspend/resume notification cleanup: {error}");
                    }
                }
                if self.session_registered {
                    if let Err(error) = WTSUnRegisterSessionNotification(self.hwnd) {
                        eprintln!("WTS cleanup: {error}");
                    }
                }
                if !self.foreground.0.is_null() && !UnhookWinEvent(self.foreground).as_bool() {
                    eprintln!("Foreground hook cleanup failed");
                }
                let _ = DestroyWindow(self.hwnd);
                let _ = UnregisterClassW(PCWSTR(self.class_name.as_ptr()), self.instance);
            }
            EVENT_WINDOW.store(0, Ordering::SeqCst);
            FOREGROUND.store(0, Ordering::SeqCst);
        }
    }

    fn post_to_hub(message: u32) -> bool {
        let raw = EVENT_WINDOW.load(Ordering::SeqCst);
        raw != 0 && unsafe { PostMessageW(HWND(raw as _), message, WPARAM(0), LPARAM(0)) }.is_ok()
    }

    unsafe extern "system" fn foreground_callback(
        _hook: HWINEVENTHOOK,
        _event: u32,
        _hwnd: HWND,
        _object: i32,
        _child: i32,
        _thread: u32,
        _time: u32,
    ) {
        let result = catch_unwind(AssertUnwindSafe(|| {
            post_to_hub(WM_TUCK_FOREGROUND);
        }));
        if result.is_err() {
            eprintln!("Foreground callback panicked");
        }
    }

    unsafe extern "system" fn location_callback(
        _hook: HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        object: i32,
        child: i32,
        _thread: u32,
        _time: u32,
    ) {
        let result = catch_unwind(AssertUnwindSafe(|| {
            if object != OBJID_WINDOW
                || child != CHILDID_SELF
                || hwnd.0 as isize != FOREGROUND.load(Ordering::SeqCst)
            {
                return;
            }
            if !LOCATION_POSTED.swap(true, Ordering::SeqCst) && !post_to_hub(WM_TUCK_LOCATION) {
                LOCATION_POSTED.store(false, Ordering::SeqCst);
            }
        }));
        if result.is_err() {
            eprintln!("Location callback panicked");
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let result = catch_unwind(AssertUnwindSafe(|| unsafe {
            if message == WM_NCCREATE {
                let create = &*(lparam.0 as *const CREATESTRUCTW);
                let context = create.lpCreateParams as *mut Context;
                if !context.is_null() {
                    (*context).hwnd = hwnd;
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, context as isize);
                return DefWindowProcW(hwnd, message, wparam, lparam);
            }

            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Context;
            if raw.is_null() {
                return DefWindowProcW(hwnd, message, wparam, lparam);
            }
            let context = &mut *raw;

            match message {
                WM_TUCK_STOP => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                WM_TUCK_FOREGROUND => {
                    crate::desktop_signals::input_state_changed();
                    context.watch_foreground();
                    context.refresh_layout();
                    LRESULT(0)
                }
                WM_TUCK_LOCATION => {
                    context.location_changed();
                    LRESULT(0)
                }
                0x031D => {
                    // WM_CLIPBOARDUPDATE
                    if context.clipboard_listener {
                        crate::clipboard_history::clipboard_updated();
                    }
                    LRESULT(0)
                }
                WM_TUCK_REFRESH => {
                    context.update_clipboard_listener();
                    context.refresh_layout();
                    LRESULT(0)
                }
                WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                    crate::telemetry::invalidate_adapters();
                    context.refresh_layout();
                    DefWindowProcW(hwnd, message, wparam, lparam)
                }
                WM_POWERBROADCAST => {
                    match wparam.0 as u32 {
                        PBT_APMSUSPEND => context.suspend(),
                        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => context.resume(),
                        PBT_POWERSETTINGCHANGE => {
                            context.power_setting(lparam.0 as *const POWERBROADCAST_SETTING)
                        }
                        _ => {}
                    }
                    LRESULT(1)
                }
                WM_WTSSESSION_CHANGE => {
                    match wparam.0 as u32 {
                        WTS_SESSION_LOCK => context.lock(),
                        WTS_SESSION_UNLOCK => context.unlock(),
                        _ => {}
                    }
                    LRESULT(0)
                }
                WM_TIMER if wparam.0 == RESUME_TIMER => {
                    let _ = KillTimer(hwnd, RESUME_TIMER);
                    context.watch_foreground();
                    context.restore_island();
                    LRESULT(0)
                }
                WM_TIMER if wparam.0 == LOCATION_TIMER => {
                    context.location_timer();
                    LRESULT(0)
                }
                WM_TIMER if wparam.0 == FULLSCREEN_TIMER => {
                    if !context.island_hidden() {
                        context.refresh_fullscreen();
                    }
                    LRESULT(0)
                }
                value if value == context.taskbar_created => {
                    // Explorer restarted: its flyout host is new and visible again.
                    crate::osd_flyout::reapply();
                    context.refresh_layout();
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }));

        match result {
            Ok(value) => value,
            Err(_) => {
                eprintln!("Lifecycle window callback panicked");
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn register_power(hwnd: HWND, guid: &windows::core::GUID) -> Option<HPOWERNOTIFY> {
        unsafe {
            RegisterPowerSettingNotification(HANDLE(hwnd.0), guid, DEVICE_NOTIFY_WINDOW_HANDLE)
                .map_err(|error| eprintln!("Power notification registration: {error}"))
                .ok()
        }
    }

    fn run(app: AppHandle) -> windows::core::Result<()> {
        let module = unsafe { GetModuleHandleW(None)? };
        let instance = HINSTANCE(module.0);
        let class_name = wide("TuckLifecycleEvents");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(windows::core::Error::from_win32());
        }

        let mut context = Box::new(Context {
            app,
            hwnd: HWND::default(),
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            locked: false,
            suspended: false,
            display_off: false,
            last_resume: None,
            location_hook: HWINEVENTHOOK::default(),
            location_pid: 0,
            location_debounce: Debouncer::new(LOCATION_QUIET, LOCATION_MAX_WAIT),
            fullscreen_timer: false,
            clipboard_listener: false,
        });
        let context_ptr = (&mut *context) as *mut Context;
        // A hidden top-level window (never shown) rather than HWND_MESSAGE: message-only
        // windows do not receive broadcasts such as WM_POWERBROADCAST or TaskbarCreated.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                PCWSTR(class_name.as_ptr()),
                w!("Tuck lifecycle events"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                HWND::default(),
                HMENU::default(),
                instance,
                Some(context_ptr.cast()),
            )?
        };
        EVENT_WINDOW.store(hwnd.0 as isize, Ordering::SeqCst);
        let session_registered = unsafe {
            WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)
                .map_err(|error| eprintln!("WTS registration: {error}"))
                .is_ok()
        };
        // Modern Standby systems only deliver suspend/resume to registered recipients.
        let suspend_resume = unsafe {
            RegisterSuspendResumeNotification(HANDLE(hwnd.0), DEVICE_NOTIFY_WINDOW_HANDLE)
                .map_err(|error| eprintln!("Suspend/resume registration: {error}"))
                .ok()
        };
        let foreground = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                HMODULE::default(),
                Some(foreground_callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if foreground.0.is_null() {
            eprintln!("Foreground hook registration failed");
        }
        let power = [
            &GUID_CONSOLE_DISPLAY_STATE,
            &GUID_ACDC_POWER_SOURCE,
            &GUID_BATTERY_PERCENTAGE_REMAINING,
        ]
        .into_iter()
        .filter_map(|guid| register_power(hwnd, guid))
        .collect();
        let _registrations = Registrations {
            hwnd,
            power,
            suspend_resume,
            foreground,
            session_registered,
            class_name,
            instance,
        };

        context.update_clipboard_listener();
        context.emit_power();
        context.watch_foreground();
        context.refresh_layout();
        let mut message = MSG::default();
        loop {
            let result = unsafe { GetMessageW(&mut message, HWND::default(), 0, 0) };
            if result.0 <= 0 {
                break;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }

    pub fn start(app: AppHandle) {
        if let Err(error) = std::thread::Builder::new()
            .name("tuck-lifecycle".into())
            .spawn(move || {
                if let Err(error) = run(app) {
                    eprintln!("Lifecycle event hub stopped: {error}");
                }
            })
        {
            eprintln!("Could not start lifecycle hub: {error}");
        }
    }

    pub fn request_refresh() -> bool {
        post_to_hub(WM_TUCK_REFRESH)
    }

    pub fn stop() {
        if EVENT_WINDOW.load(Ordering::SeqCst) != 0 && !post_to_hub(WM_TUCK_STOP) {
            eprintln!("Lifecycle shutdown signal failed");
        }
    }
}

pub fn start(app: AppHandle) {
    #[cfg(windows)]
    native::start(app);
    #[cfg(not(windows))]
    let _ = app;
}

/// Asks the lifecycle thread to re-evaluate layout and fullscreen state.
/// Returns false when the hub is not running, so the caller can refresh directly.
pub(crate) fn request_refresh() -> bool {
    #[cfg(windows)]
    let posted = native::request_refresh();
    #[cfg(not(windows))]
    let posted = false;
    posted
}

pub fn stop() {
    #[cfg(windows)]
    native::stop();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn debounce_fires_after_quiet_period() {
        let start = Instant::now();
        let mut debounce = Debouncer::new(ms(200), ms(1000));
        assert_eq!(debounce.poll(start), DebounceStep::Idle);
        assert!(debounce.event(start));
        assert!(!debounce.event(start + ms(50)));
        assert_eq!(debounce.poll(start + ms(200)), DebounceStep::Wait(ms(50)));
        assert_eq!(debounce.poll(start + ms(250)), DebounceStep::Fire);
        assert_eq!(debounce.poll(start + ms(300)), DebounceStep::Idle);
        assert!(
            debounce.event(start + ms(400)),
            "a new burst re-arms the timer"
        );
    }

    #[test]
    fn debounce_caps_continuous_bursts() {
        let start = Instant::now();
        let mut debounce = Debouncer::new(ms(200), ms(1000));
        debounce.event(start);
        for step in 1..=9 {
            let now = start + ms(step * 100);
            debounce.event(now);
            assert!(matches!(debounce.poll(now), DebounceStep::Wait(_)));
        }
        assert_eq!(debounce.poll(start + ms(950)), DebounceStep::Wait(ms(50)));
        debounce.event(start + ms(990));
        assert_eq!(debounce.poll(start + ms(1000)), DebounceStep::Fire);
    }

    #[test]
    fn resume_dedupe_only_skips_recent_repeats() {
        let now = Instant::now();
        assert!(!is_duplicate_resume(true, Some(now), now));
        assert!(!is_duplicate_resume(false, None, now));
        assert!(is_duplicate_resume(false, Some(now), now + ms(500)));
        assert!(!is_duplicate_resume(false, Some(now), now + RESUME_DEDUPE));
    }
}

#[cfg(test)]
mod clipboard_foreground_tests {
    #[test]
    fn enabled_clipboard_survives_foreground_switch_and_receives_notification() {
        use std::cell::Cell;
        let clipboard_registered = Cell::new(false);
        let mut listener_enabled = false;
        super::update_clipboard_registration(&mut listener_enabled, true, |enabled| {
            clipboard_registered.set(enabled);
            Ok::<(), ()>(())
        })
        .unwrap();
        let location_registered = Cell::new(7u32);
        let (mut pid, mut hook) = (7, 7);
        super::rebind_foreground(
            9,
            &mut pid,
            &mut hook,
            |h| h != 0,
            |_| location_registered.set(0),
            |next| {
                location_registered.set(next);
                next
            },
        );
        assert_eq!(location_registered.get(), 9);
        assert!(clipboard_registered.get());
        let notifications = Cell::new(0);
        if clipboard_registered.get() {
            notifications.set(notifications.get() + 1);
        }
        assert_eq!(notifications.get(), 1);
        // Disabling/destroying the window owns the only clipboard removal.
        super::update_clipboard_registration(&mut listener_enabled, false, |enabled| {
            clipboard_registered.set(enabled);
            Ok::<(), ()>(())
        })
        .unwrap();
        if clipboard_registered.get() {
            notifications.set(notifications.get() + 1);
        }
        assert_eq!(notifications.get(), 1);
    }
}
