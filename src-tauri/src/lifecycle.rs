//! One Win32 message-only window owns lifecycle notifications for the island.
//! It replaces periodic power/display/fullscreen polling and centralizes cleanup.

use tauri::AppHandle;

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        panic::{catch_unwind, AssertUnwindSafe},
        sync::atomic::{AtomicIsize, Ordering},
    };
    use tauri::{Emitter, Manager};
    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::{HANDLE, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, WPARAM},
            System::{
                LibraryLoader::GetModuleHandleW,
                Power::{
                    RegisterPowerSettingNotification, UnregisterPowerSettingNotification,
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
            },
            UI::{
                Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
                WindowsAndMessaging::{
                    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
                    GetWindowLongPtrW, KillTimer, PostMessageW, PostQuitMessage, RegisterClassW,
                    RegisterWindowMessageW, SetTimer, SetWindowLongPtrW, TranslateMessage,
                    UnregisterClassW, CREATESTRUCTW, DEVICE_NOTIFY_WINDOW_HANDLE,
                    EVENT_SYSTEM_FOREGROUND, GWLP_USERDATA, HMENU, HWND_MESSAGE, MSG,
                    PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE,
                    WINDOW_EX_STYLE, WINDOW_STYLE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
                    WM_APP, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_NCCREATE, WM_POWERBROADCAST,
                    WM_SETTINGCHANGE, WM_TIMER, WM_WTSSESSION_CHANGE, WNDCLASSW, WTS_SESSION_LOCK,
                    WTS_SESSION_UNLOCK,
                },
            },
        },
    };

    const WM_TUCK_FOREGROUND: u32 = WM_APP + 41;
    const WM_TUCK_STOP: u32 = WM_APP + 42;
    const RESUME_TIMER: usize = 1;
    static EVENT_WINDOW: AtomicIsize = AtomicIsize::new(0);

    struct Context {
        app: AppHandle,
        taskbar_created: u32,
        locked: bool,
        suspended: bool,
        display_off: bool,
    }

    impl Context {
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
                let _ = win.set_ignore_cursor_events(true);
                let _ = win.hide();
            }
        }

        fn restore_island(&self) {
            if self.island_hidden() {
                return;
            }
            if let Some(win) = self.app.get_webview_window("main") {
                let _ = crate::system::center_on_primary_monitor(&win);
                let _ = win.show();
                crate::system::reassert_topmost(&win);
            }
            crate::system::refresh_window_state(&self.app);
        }

        fn refresh_layout(&self) {
            if let Some(win) = self.app.get_webview_window("main") {
                let _ = crate::system::center_on_primary_monitor(&win);
                if !self.island_hidden() {
                    crate::system::reassert_topmost(&win);
                }
            }
            crate::system::refresh_window_state(&self.app);
        }

        fn suspend(&mut self) {
            self.suspended = true;
            crate::system::set_cursor_pause(crate::system::CURSOR_PAUSE_SUSPEND, true);
            self.hide_island();
            crate::media::suspend();
            crate::desktop_signals::suspend();
        }

        fn resume(&mut self) {
            self.suspended = false;
            crate::system::set_cursor_pause(crate::system::CURSOR_PAUSE_SUSPEND, false);
            crate::media::resume();
            crate::desktop_signals::resume();
            self.emit_power();
            self.restore_island();
        }

        fn lock(&mut self) {
            self.locked = true;
            crate::system::set_cursor_pause(crate::system::CURSOR_PAUSE_SESSION, true);
            self.hide_island();
        }

        fn unlock(&mut self, hwnd: HWND) {
            self.locked = false;
            crate::system::set_cursor_pause(crate::system::CURSOR_PAUSE_SESSION, false);
            unsafe {
                let _ = SetTimer(hwnd, RESUME_TIMER, 150, None);
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

    struct Registrations {
        hwnd: HWND,
        power: Vec<HPOWERNOTIFY>,
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
        }
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
            let raw = EVENT_WINDOW.load(Ordering::SeqCst);
            if raw != 0 {
                unsafe {
                    let _ = PostMessageW(HWND(raw as _), WM_TUCK_FOREGROUND, WPARAM(0), LPARAM(0));
                }
            }
        }));
        if result.is_err() {
            eprintln!("Foreground callback panicked");
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
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
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
                    context.refresh_layout();
                    LRESULT(0)
                }
                WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
                    context.refresh_layout();
                    LRESULT(0)
                }
                WM_POWERBROADCAST => {
                    match wparam.0 as u32 {
                        PBT_APMSUSPEND => context.suspend(),
                        PBT_APMRESUMEAUTOMATIC => context.resume(),
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
                        WTS_SESSION_UNLOCK => context.unlock(hwnd),
                        _ => {}
                    }
                    LRESULT(0)
                }
                WM_TIMER if wparam.0 == RESUME_TIMER => {
                    let _ = KillTimer(hwnd, RESUME_TIMER);
                    context.restore_island();
                    LRESULT(0)
                }
                value if value == context.taskbar_created => {
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
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            locked: false,
            suspended: false,
            display_off: false,
        });
        let context_ptr = (&mut *context) as *mut Context;
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_name.as_ptr()),
                w!("Tuck lifecycle events"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
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
            foreground,
            session_registered,
            class_name,
            instance,
        };

        context.emit_power();
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

    pub fn stop() {
        let raw = EVENT_WINDOW.load(Ordering::SeqCst);
        if raw != 0 {
            unsafe {
                if let Err(error) = PostMessageW(HWND(raw as _), WM_TUCK_STOP, WPARAM(0), LPARAM(0))
                {
                    eprintln!("Lifecycle shutdown signal: {error}");
                }
            }
        }
    }
}

pub fn start(app: AppHandle) {
    #[cfg(windows)]
    native::start(app);
    #[cfg(not(windows))]
    let _ = app;
}

pub fn stop() {
    #[cfg(windows)]
    native::stop();
}
