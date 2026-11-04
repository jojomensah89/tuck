//! Detects "both Shift keys held" with Raw Input instead of polling.
//!
//! The monitor thread starts the first time the shortcut is turned on. While the shortcut
//! is off it blocks in `GetMessageW` with no keyboard registration, so it never wakes.
//! While it is on, it wakes once per key event. A timer runs only during a hold, because
//! a release can be invisible (elevated foreground window, secure desktop, sleep).
//!
//! Raw Input was chosen over `WH_KEYBOARD_LL`: it cannot delay anyone's typing and adds
//! no callback to the system input path. Each event is reduced to "was this a physical
//! Shift key"; nothing else is read, stored or logged.
//!
//! Windows allows one Raw Input target per device class in a process, so this replaces
//! tao's keyboard registration. tao only uses it for `DeviceEvent::Key`, which Tuck does
//! not consume. Mouse device events are a separate class and stay with tao.

/// What the monitor reports to its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum HoldChange {
    Pressed,
    Released,
    Unavailable(String),
}

pub(super) type Handler = Box<dyn FnMut(HoldChange) + Send>;

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShiftKey {
    Left,
    Right,
}

const VK_SHIFT_CODE: u16 = 0x10;
const SCAN_LEFT_SHIFT: u16 = 0x2A;
const SCAN_RIGHT_SHIFT: u16 = 0x36;
const RI_KEY_BREAK: u16 = 1;
const RI_KEY_E0: u16 = 2;

/// Returns the physical Shift key an event belongs to and whether it went down.
/// `VKey` must be checked as well as the scan code: Unicode text sent with `SendInput`,
/// including Tuck's own insertion, can carry a code unit in `MakeCode` (`*` is 0x2A).
/// E0-prefixed Shift events are the fake ones the keyboard sends around navigation keys.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn classify_shift(vkey: u16, make_code: u16, flags: u16) -> Option<(ShiftKey, bool)> {
    if vkey != VK_SHIFT_CODE || flags & RI_KEY_E0 != 0 {
        return None;
    }
    let key = match make_code {
        SCAN_LEFT_SHIFT => ShiftKey::Left,
        SCAN_RIGHT_SHIFT => ShiftKey::Right,
        _ => return None,
    };
    Some((key, flags & RI_KEY_BREAK == 0))
}

/// Key repeat arrives as more key-downs, so only an edge produces a change.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn hold_transition(holding: bool, both_down: bool) -> Option<HoldChange> {
    match (holding, both_down) {
        (false, true) => Some(HoldChange::Pressed),
        (true, false) => Some(HoldChange::Released),
        _ => None,
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        panic::{catch_unwind, AssertUnwindSafe},
        sync::atomic::{AtomicBool, AtomicIsize, Ordering},
    };
    use windows::{
        core::w,
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LSHIFT, VK_RSHIFT},
                Input::{
                    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
                    RAWINPUTHEADER, RIDEV_INPUTSINK, RIDEV_REMOVE, RID_INPUT, RIM_TYPEKEYBOARD,
                },
                WindowsAndMessaging::{
                    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
                    GetWindowLongPtrW, KillTimer, PostMessageW, PostQuitMessage, RegisterClassW,
                    SetTimer, SetWindowLongPtrW, CREATESTRUCTW, GWLP_USERDATA, HMENU, HWND_MESSAGE,
                    MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_INPUT, WM_NCCREATE, WM_TIMER,
                    WNDCLASSW,
                },
            },
        },
    };

    const WM_TUCK_SYNC: u32 = WM_APP + 51;
    const WM_TUCK_STOP: u32 = WM_APP + 52;
    const HOLD_TIMER: usize = 1;
    const HOLD_RECHECK_MS: u32 = 100;
    const USAGE_PAGE_GENERIC: u16 = 0x01;
    const USAGE_KEYBOARD: u16 = 0x06;

    static WINDOW: AtomicIsize = AtomicIsize::new(0);
    static WANTED: AtomicBool = AtomicBool::new(false);
    static STARTED: AtomicBool = AtomicBool::new(false);

    #[cfg(test)]
    pub(super) static KEY_EVENTS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    pub(super) type IsDown = fn(ShiftKey) -> bool;

    fn async_is_down(key: ShiftKey) -> bool {
        let vk = match key {
            ShiftKey::Left => VK_LSHIFT,
            ShiftKey::Right => VK_RSHIFT,
        };
        unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
    }

    struct Monitor {
        handler: Handler,
        is_down: IsDown,
        hwnd: HWND,
        registered: bool,
        holding: bool,
    }

    impl Monitor {
        fn set_holding(&mut self, both_down: bool) {
            let Some(change) = hold_transition(self.holding, both_down) else {
                return;
            };
            self.holding = both_down;
            unsafe {
                if both_down {
                    SetTimer(self.hwnd, HOLD_TIMER, HOLD_RECHECK_MS, None);
                } else {
                    let _ = KillTimer(self.hwnd, HOLD_TIMER);
                }
            }
            (self.handler)(change);
        }

        fn sync(&mut self) {
            let wanted = WANTED.load(Ordering::SeqCst);
            if wanted == self.registered {
                return;
            }
            let device = RAWINPUTDEVICE {
                usUsagePage: USAGE_PAGE_GENERIC,
                usUsage: USAGE_KEYBOARD,
                dwFlags: if wanted {
                    RIDEV_INPUTSINK
                } else {
                    RIDEV_REMOVE
                },
                hwndTarget: if wanted { self.hwnd } else { HWND::default() },
            };
            let result = unsafe {
                RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32)
            };
            match result {
                Ok(()) => self.registered = wanted,
                Err(error) => (self.handler)(HoldChange::Unavailable(format!(
                    "Holding both Shift keys is unavailable: {error}"
                ))),
            }
            if !wanted {
                self.set_holding(false);
            }
        }

        fn key_event(&mut self, lparam: LPARAM) {
            #[cfg(test)]
            KEY_EVENTS.fetch_add(1, Ordering::SeqCst);
            let Some((key, down)) = read_shift_event(lparam) else {
                return;
            };
            // The event is authoritative for its own key. The other key comes from the
            // live state, so a release this window never saw cannot leave it stuck down.
            let other = match key {
                ShiftKey::Left => ShiftKey::Right,
                ShiftKey::Right => ShiftKey::Left,
            };
            self.set_holding(down && (self.is_down)(other));
        }

        fn recheck_hold(&mut self) {
            let both = (self.is_down)(ShiftKey::Left) && (self.is_down)(ShiftKey::Right);
            self.set_holding(both);
        }
    }

    fn read_shift_event(lparam: LPARAM) -> Option<(ShiftKey, bool)> {
        let mut data = RAWINPUT::default();
        let mut size = std::mem::size_of::<RAWINPUT>() as u32;
        let copied = unsafe {
            GetRawInputData(
                HRAWINPUT(lparam.0 as _),
                RID_INPUT,
                Some((&mut data as *mut RAWINPUT).cast()),
                &mut size,
                std::mem::size_of::<RAWINPUTHEADER>() as u32,
            )
        };
        if copied == u32::MAX || data.header.dwType != RIM_TYPEKEYBOARD.0 {
            return None;
        }
        let keyboard = unsafe { data.data.keyboard };
        classify_shift(keyboard.VKey, keyboard.MakeCode, keyboard.Flags)
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
                return;
            }
            let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Monitor;
            if raw.is_null() {
                return;
            }
            let monitor = &mut *raw;
            match message {
                WM_INPUT => monitor.key_event(lparam),
                WM_TIMER if wparam.0 == HOLD_TIMER => monitor.recheck_hold(),
                WM_TUCK_SYNC => monitor.sync(),
                WM_TUCK_STOP => PostQuitMessage(0),
                _ => {}
            }
        }));
        if result.is_err() {
            eprintln!("Dual-shift monitor callback panicked");
        }
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    fn run(handler: Handler, is_down: IsDown) -> windows::core::Result<()> {
        let module = unsafe { GetModuleHandleW(None)? };
        let instance = HINSTANCE(module.0);
        let class_name = w!("TuckDualShift");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(windows::core::Error::from_win32());
        }
        let mut monitor = Box::new(Monitor {
            handler,
            is_down,
            hwnd: HWND::default(),
            registered: false,
            holding: false,
        });
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                w!("Tuck dual shift"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                HMENU::default(),
                instance,
                Some((&mut *monitor as *mut Monitor).cast()),
            )?
        };
        monitor.hwnd = hwnd;
        WINDOW.store(hwnd.0 as isize, Ordering::SeqCst);
        // `set_enabled` stores WANTED before reading WINDOW, so a change made while this
        // thread was starting is either posted or picked up by this first sync.
        monitor.sync();
        let mut message = MSG::default();
        loop {
            if unsafe { GetMessageW(&mut message, HWND::default(), 0, 0) }.0 <= 0 {
                break;
            }
            unsafe { DispatchMessageW(&message) };
        }
        WANTED.store(false, Ordering::SeqCst);
        monitor.sync();
        WINDOW.store(0, Ordering::SeqCst);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        Ok(())
    }

    fn post(message: u32) {
        let raw = WINDOW.load(Ordering::SeqCst);
        if raw != 0 {
            unsafe {
                let _ = PostMessageW(HWND(raw as _), message, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub(super) fn set_enabled_with(
        enabled: bool,
        make_handler: impl FnOnce() -> Handler,
        is_down: IsDown,
    ) {
        WANTED.store(enabled, Ordering::SeqCst);
        if enabled && !STARTED.swap(true, Ordering::SeqCst) {
            let handler = make_handler();
            let spawned = std::thread::Builder::new()
                .name("tuck-dual-shift".into())
                .spawn(move || {
                    if let Err(error) = run(handler, is_down) {
                        eprintln!("Dual-shift monitor stopped: {error}");
                    }
                });
            if let Err(error) = spawned {
                eprintln!("Could not start dual-shift monitor: {error}");
                STARTED.store(false, Ordering::SeqCst);
            }
            return;
        }
        post(WM_TUCK_SYNC);
    }

    pub(in super::super) fn set_enabled(enabled: bool, make_handler: impl FnOnce() -> Handler) {
        set_enabled_with(enabled, make_handler, async_is_down);
    }

    pub(in super::super) fn stop() {
        post(WM_TUCK_STOP);
    }
}

#[cfg(windows)]
pub(super) use native::{set_enabled, stop};

#[cfg(not(windows))]
pub(super) fn set_enabled(_enabled: bool, _make_handler: impl FnOnce() -> Handler) {}

#[cfg(not(windows))]
pub(super) fn stop() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_physical_shift_keys_count() {
        assert_eq!(
            classify_shift(0x10, SCAN_LEFT_SHIFT, 0),
            Some((ShiftKey::Left, true))
        );
        assert_eq!(
            classify_shift(0x10, SCAN_RIGHT_SHIFT, RI_KEY_BREAK),
            Some((ShiftKey::Right, false))
        );
        // Fake Shift sent around navigation keys
        assert_eq!(classify_shift(0x10, SCAN_LEFT_SHIFT, RI_KEY_E0), None);
        // Unicode insertion: VK_PACKET whose scan field holds '*' and '6'
        assert_eq!(classify_shift(0xE7, 0x2A, 0), None);
        assert_eq!(classify_shift(0xE7, 0x36, RI_KEY_BREAK), None);
        assert_eq!(classify_shift(0x41, 0x1E, 0), None);
    }

    #[test]
    fn hold_changes_only_on_edges() {
        assert_eq!(hold_transition(false, true), Some(HoldChange::Pressed));
        assert_eq!(hold_transition(true, true), None, "key repeat");
        assert_eq!(hold_transition(true, false), Some(HoldChange::Released));
        assert_eq!(hold_transition(false, false), None);
    }

    /// Injects one real Shift key, so it needs an interactive desktop and is not run in CI.
    /// Only one key is pressed, so a running copy of Tuck is not triggered: the second key
    /// is simulated through `is_down`.
    #[cfg(windows)]
    #[test]
    #[ignore = "injects a Shift key; run on a desktop with --ignored"]
    fn raw_input_reaches_the_monitor_and_stops_when_disabled() {
        use native::KEY_EVENTS;
        use std::sync::atomic::Ordering;
        use std::sync::mpsc;
        use std::time::Duration;
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
            KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
        };

        fn left_shift(flags: KEYBD_EVENT_FLAGS) {
            let input = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wScan: SCAN_LEFT_SHIFT,
                        dwFlags: KEYEVENTF_SCANCODE | flags,
                        ..Default::default()
                    },
                },
            };
            let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
            assert_eq!(sent, 1);
        }

        let (tx, rx) = mpsc::channel();
        native::set_enabled_with(
            true,
            || Box::new(move |change| tx.send(change).unwrap()),
            |_| true,
        );
        std::thread::sleep(Duration::from_millis(300));

        left_shift(KEYBD_EVENT_FLAGS(0));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            HoldChange::Pressed
        );
        left_shift(KEYEVENTF_KEYUP);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            HoldChange::Released
        );
        assert!(KEY_EVENTS.load(Ordering::SeqCst) >= 2);

        native::set_enabled_with(false, || unreachable!("already started"), |_| true);
        std::thread::sleep(Duration::from_millis(200));
        let before = KEY_EVENTS.load(Ordering::SeqCst);
        left_shift(KEYBD_EVENT_FLAGS(0));
        left_shift(KEYEVENTF_KEYUP);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            KEY_EVENTS.load(Ordering::SeqCst),
            before,
            "disabled means no key events reach the thread"
        );
        native::stop();
    }
}
