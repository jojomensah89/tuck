//! Detects a held modifier-only shortcut with Raw Input instead of polling: both Alt keys
//! together, both Shift keys together, or Right Alt alone.
//!
//! The monitor thread starts the first time the shortcut is turned on. While the shortcut
//! is off it blocks in `GetMessageW` with no keyboard registration, so it never wakes.
//! While it is on, it wakes once per key event. A hold must last `MIN_HOLD_MS`, timed by a
//! one-shot timer, so rolling between Shift keys while typing starts nothing. A second
//! timer runs only during a hold, because a release can be invisible (elevated foreground
//! window, secure desktop, sleep).
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
    /// The keyboard registration works, so any earlier problem is over.
    Available,
}

pub(super) type Handler = Box<dyn FnMut(HoldChange) + Send>;

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShiftKey {
    Left,
    Right,
}

/// Which modifier-only hold the monitor watches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HoldKey {
    BothShifts,
    RightAlt,
    BothAlts,
}

const VK_SHIFT_CODE: u16 = 0x10;
const VK_CONTROL_CODE: u16 = 0x11;
const VK_MENU_CODE: u16 = 0x12;
const SCAN_ALT: u16 = 0x38;
/// Unassigned virtual key. Pressed once during a Right Alt hold so that releasing Alt is
/// not a lone Alt tap, which would move focus to the foreground app's menu bar.
#[cfg_attr(not(windows), allow(dead_code))]
const VK_MASK: u16 = 0xE8;
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

/// A Right Alt event, as (is it Right Alt, went down). Left Alt has no E0 prefix.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn classify_right_alt(vkey: u16, make_code: u16, flags: u16) -> Option<bool> {
    (vkey == VK_MENU_CODE && make_code == SCAN_ALT && flags & RI_KEY_E0 != 0)
        .then_some(flags & RI_KEY_BREAK == 0)
}

/// An Alt event, as (is it the right one, went down).
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn classify_alt(vkey: u16, make_code: u16, flags: u16) -> Option<(bool, bool)> {
    (vkey == VK_MENU_CODE && make_code == SCAN_ALT)
        .then_some((flags & RI_KEY_E0 != 0, flags & RI_KEY_BREAK == 0))
}

/// A key press that means Right Alt is part of a chord, not a hold. Ctrl is left out
/// because AltGr layouts send a fake Left Ctrl with every Right Alt event, and the mask
/// key is Tuck's own.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn breaks_right_alt_hold(vkey: u16, flags: u16) -> bool {
    flags & RI_KEY_BREAK == 0 && !matches!(vkey, VK_MENU_CODE | VK_CONTROL_CODE | VK_MASK)
}

/// How long both keys must stay down. Rolling between Shift keys while typing "USA"
/// overlaps them for far less than this.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
const MIN_HOLD_MS: u32 = 200;

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hold {
    Idle,
    Arming,
    Held,
}

/// A key event. Key repeat arrives as more key-downs, so only a change does anything.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn hold_after_keys(hold: Hold, both_down: bool) -> (Hold, Option<HoldChange>) {
    match (hold, both_down) {
        (Hold::Idle, true) => (Hold::Arming, None),
        (Hold::Arming, false) => (Hold::Idle, None),
        (Hold::Held, false) => (Hold::Idle, Some(HoldChange::Released)),
        (unchanged, _) => (unchanged, None),
    }
}

/// Another key went down. While arming that makes it a chord such as Right Alt+Tab, so
/// nothing starts; an established hold keeps recording.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn hold_after_other_key(hold: Hold) -> (Hold, Option<HoldChange>) {
    match hold {
        Hold::Arming => (Hold::Idle, None),
        unchanged => (unchanged, None),
    }
}

/// The one-shot timer ran out. The keys still count only if they are still down.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn hold_after_arm_timer(hold: Hold, both_down: bool) -> (Hold, Option<HoldChange>) {
    match (hold, both_down) {
        (Hold::Arming, true) => (Hold::Held, Some(HoldChange::Pressed)),
        (Hold::Arming, false) => (Hold::Idle, None),
        (unchanged, _) => (unchanged, None),
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering},
        Arc, Mutex,
    };
    use windows::{
        core::w,
        Win32::{
            Foundation::{
                GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
            },
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Input::KeyboardAndMouse::{
                    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                    KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_LMENU, VK_LSHIFT, VK_RMENU, VK_RSHIFT,
                },
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
    const ARM_TIMER: usize = 2;
    const HOLD_RECHECK_MS: u32 = 100;
    const USAGE_PAGE_GENERIC: u16 = 0x01;
    const USAGE_KEYBOARD: u16 = 0x06;

    static WINDOW: AtomicIsize = AtomicIsize::new(0);
    static WANTED: AtomicBool = AtomicBool::new(false);
    static STARTED: AtomicBool = AtomicBool::new(false);
    /// 0 watches both Shift keys, 1 Right Alt, 2 both Alt keys.
    static KEY: AtomicU8 = AtomicU8::new(0);

    pub(in super::super) fn set_key(key: HoldKey) {
        let value = match key {
            HoldKey::BothShifts => 0,
            HoldKey::RightAlt => 1,
            HoldKey::BothAlts => 2,
        };
        KEY.store(value, Ordering::SeqCst);
    }

    fn key() -> HoldKey {
        match KEY.load(Ordering::SeqCst) {
            1 => HoldKey::RightAlt,
            2 => HoldKey::BothAlts,
            _ => HoldKey::BothShifts,
        }
    }

    fn right_alt_is_down() -> bool {
        unsafe { GetAsyncKeyState(VK_RMENU.0 as i32) < 0 }
    }

    fn left_alt_is_down() -> bool {
        unsafe { GetAsyncKeyState(VK_LMENU.0 as i32) < 0 }
    }

    fn send_mask_key() {
        let key = |up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(VK_MASK),
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                    ..Default::default()
                },
            },
        };
        unsafe {
            SendInput(
                &[key(false), key(true)],
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }

    #[cfg(test)]
    pub(super) static KEY_EVENTS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    pub(super) type IsDown = fn(ShiftKey) -> bool;
    pub(super) type SharedHandler = Arc<Mutex<Handler>>;
    /// `run`, or a stand-in in tests that need it to fail.
    pub(super) type Runner = fn(&SharedHandler, IsDown) -> windows::core::Result<()>;

    fn report(handler: &SharedHandler, change: HoldChange) {
        if let Ok(mut handler) = handler.lock() {
            handler(change);
        }
    }

    fn async_is_down(key: ShiftKey) -> bool {
        let vk = match key {
            ShiftKey::Left => VK_LSHIFT,
            ShiftKey::Right => VK_RSHIFT,
        };
        unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
    }

    struct Monitor {
        handler: SharedHandler,
        is_down: IsDown,
        hwnd: HWND,
        registered: bool,
        hold: Hold,
    }

    impl Monitor {
        fn both_down(&self) -> bool {
            match key() {
                HoldKey::BothShifts => {
                    (self.is_down)(ShiftKey::Left) && (self.is_down)(ShiftKey::Right)
                }
                HoldKey::RightAlt => right_alt_is_down(),
                HoldKey::BothAlts => left_alt_is_down() && right_alt_is_down(),
            }
        }

        fn apply(&mut self, next: (Hold, Option<HoldChange>)) {
            let (hold, change) = next;
            if hold != self.hold {
                unsafe {
                    // Each timer runs only in the state that needs it.
                    match (self.hold, hold) {
                        (Hold::Idle, Hold::Arming) => {
                            SetTimer(self.hwnd, ARM_TIMER, MIN_HOLD_MS, None);
                        }
                        (Hold::Arming, Hold::Held) => {
                            let _ = KillTimer(self.hwnd, ARM_TIMER);
                            if matches!(key(), HoldKey::RightAlt | HoldKey::BothAlts) {
                                send_mask_key();
                            }
                            SetTimer(self.hwnd, HOLD_TIMER, HOLD_RECHECK_MS, None);
                        }
                        (Hold::Arming, Hold::Idle) => {
                            let _ = KillTimer(self.hwnd, ARM_TIMER);
                        }
                        (Hold::Held, Hold::Idle) => {
                            let _ = KillTimer(self.hwnd, HOLD_TIMER);
                        }
                        _ => {}
                    }
                }
                self.hold = hold;
            }
            if let Some(change) = change {
                report(&self.handler, change);
            }
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
                Ok(()) => {
                    self.registered = wanted;
                    report(&self.handler, HoldChange::Available);
                }
                Err(error) => report(
                    &self.handler,
                    HoldChange::Unavailable(format!("This hold shortcut is unavailable: {error}")),
                ),
            }
            if !wanted {
                self.apply(hold_after_keys(self.hold, false));
            }
        }

        fn key_event(&mut self, lparam: LPARAM) {
            #[cfg(test)]
            KEY_EVENTS.fetch_add(1, Ordering::SeqCst);
            let Some((vkey, make_code, flags)) = read_key_event(lparam) else {
                return;
            };
            if key() == HoldKey::BothAlts {
                if let Some((right, down)) = classify_alt(vkey, make_code, flags) {
                    // This event is authoritative for its own key; the other comes from the
                    // live state, so a release this window never saw can't leave it stuck.
                    let other = if right {
                        left_alt_is_down()
                    } else {
                        right_alt_is_down()
                    };
                    self.apply(hold_after_keys(self.hold, down && other));
                } else if breaks_right_alt_hold(vkey, flags) {
                    self.apply(hold_after_other_key(self.hold));
                }
                return;
            }
            if key() == HoldKey::RightAlt {
                if let Some(down) = classify_right_alt(vkey, make_code, flags) {
                    self.apply(hold_after_keys(self.hold, down));
                } else if breaks_right_alt_hold(vkey, flags) {
                    self.apply(hold_after_other_key(self.hold));
                }
                return;
            }
            let Some((key, down)) = classify_shift(vkey, make_code, flags) else {
                return;
            };
            // The event is authoritative for its own key. The other key comes from the
            // live state, so a release this window never saw cannot leave it stuck down.
            let other = match key {
                ShiftKey::Left => ShiftKey::Right,
                ShiftKey::Right => ShiftKey::Left,
            };
            self.apply(hold_after_keys(self.hold, down && (self.is_down)(other)));
        }

        fn recheck_hold(&mut self) {
            self.apply(hold_after_keys(self.hold, self.both_down()));
        }

        fn arm_timer_ran_out(&mut self) {
            self.apply(hold_after_arm_timer(self.hold, self.both_down()));
        }
    }

    fn read_key_event(lparam: LPARAM) -> Option<(u16, u16, u16)> {
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
        Some((keyboard.VKey, keyboard.MakeCode, keyboard.Flags))
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe {
            if message == WM_NCCREATE {
                let create = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            } else {
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Monitor;
                if !raw.is_null() {
                    let monitor = &mut *raw;
                    match message {
                        WM_INPUT => monitor.key_event(lparam),
                        WM_TIMER if wparam.0 == HOLD_TIMER => monitor.recheck_hold(),
                        WM_TIMER if wparam.0 == ARM_TIMER => monitor.arm_timer_ran_out(),
                        WM_TUCK_SYNC => monitor.sync(),
                        WM_TUCK_STOP => PostQuitMessage(0),
                        _ => {}
                    }
                }
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
    }

    fn run(handler: &SharedHandler, is_down: IsDown) -> windows::core::Result<()> {
        let module = unsafe { GetModuleHandleW(None)? };
        let instance = HINSTANCE(module.0);
        let class_name = w!("TuckDualShift");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name,
            ..Default::default()
        };
        // A retry after a failed start finds the class already registered.
        if unsafe { RegisterClassW(&class) } == 0
            && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
        {
            return Err(windows::core::Error::from_win32());
        }
        let mut monitor = Box::new(Monitor {
            handler: handler.clone(),
            is_down,
            hwnd: HWND::default(),
            registered: false,
            hold: Hold::Idle,
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

    #[cfg(test)]
    pub(super) fn run_for_test(
        handler: &SharedHandler,
        is_down: IsDown,
    ) -> windows::core::Result<()> {
        run(handler, is_down)
    }

    fn post(message: u32) {
        let raw = WINDOW.load(Ordering::SeqCst);
        if raw != 0 {
            unsafe {
                let _ = PostMessageW(HWND(raw as _), message, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// A failed start clears STARTED, so the next attempt tries again instead of posting to
    /// a window that does not exist, and tells the owner why.
    pub(super) fn set_enabled_with(
        enabled: bool,
        make_handler: impl FnOnce() -> Handler,
        is_down: IsDown,
        runner: Runner,
    ) {
        WANTED.store(enabled, Ordering::SeqCst);
        if enabled && !STARTED.swap(true, Ordering::SeqCst) {
            let handler: SharedHandler = Arc::new(Mutex::new(make_handler()));
            let thread_handler = handler.clone();
            let spawned = std::thread::Builder::new()
                .name("tuck-dual-shift".into())
                .spawn(move || {
                    if let Err(error) = runner(&thread_handler, is_down) {
                        WINDOW.store(0, Ordering::SeqCst);
                        STARTED.store(false, Ordering::SeqCst);
                        report(
                            &thread_handler,
                            HoldChange::Unavailable(format!(
                                "This hold shortcut is unavailable: {error}"
                            )),
                        );
                    }
                });
            if let Err(error) = spawned {
                STARTED.store(false, Ordering::SeqCst);
                report(
                    &handler,
                    HoldChange::Unavailable(format!("This hold shortcut is unavailable: {error}")),
                );
            }
            return;
        }
        post(WM_TUCK_SYNC);
    }

    pub(in super::super) fn set_enabled(enabled: bool, make_handler: impl FnOnce() -> Handler) {
        set_enabled_with(enabled, make_handler, async_is_down, run);
    }

    pub(in super::super) fn stop() {
        post(WM_TUCK_STOP);
    }
}

#[cfg(windows)]
pub(super) use native::{set_enabled, set_key, stop};

#[cfg(not(windows))]
pub(super) fn set_enabled(_enabled: bool, _make_handler: impl FnOnce() -> Handler) {}

#[cfg(not(windows))]
pub(super) fn stop() {}

#[cfg(not(windows))]
pub(super) fn set_key(_key: HoldKey) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_alt_keys_are_told_apart() {
        assert_eq!(classify_alt(0x12, SCAN_ALT, 0), Some((false, true)));
        assert_eq!(classify_alt(0x12, SCAN_ALT, RI_KEY_E0), Some((true, true)));
        assert_eq!(
            classify_alt(0x12, SCAN_ALT, RI_KEY_E0 | RI_KEY_BREAK),
            Some((true, false))
        );
        assert_eq!(classify_alt(0x10, SCAN_LEFT_SHIFT, 0), None);
    }

    #[test]
    fn only_the_right_alt_key_counts() {
        assert_eq!(classify_right_alt(0x12, SCAN_ALT, RI_KEY_E0), Some(true));
        assert_eq!(
            classify_right_alt(0x12, SCAN_ALT, RI_KEY_E0 | RI_KEY_BREAK),
            Some(false)
        );
        // Left Alt has no E0 prefix.
        assert_eq!(classify_right_alt(0x12, SCAN_ALT, 0), None);
        assert_eq!(classify_right_alt(0x10, SCAN_LEFT_SHIFT, RI_KEY_E0), None);
    }

    #[test]
    fn a_chord_with_right_alt_never_starts_dictation() {
        // Right Alt+Tab: Tab goes down while the hold is still arming.
        assert!(breaks_right_alt_hold(0x09, 0));
        assert_eq!(hold_after_other_key(Hold::Arming), (Hold::Idle, None));
        // Once recording, typing does not end it.
        assert_eq!(hold_after_other_key(Hold::Held), (Hold::Held, None));
        // Key releases, Alt repeats, AltGr's fake Ctrl and Tuck's own mask key are not chords.
        assert!(!breaks_right_alt_hold(0x09, RI_KEY_BREAK));
        assert!(!breaks_right_alt_hold(0x12, 0));
        assert!(!breaks_right_alt_hold(0x11, 0));
        assert!(!breaks_right_alt_hold(VK_MASK, 0));
    }

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
    fn rolling_between_shift_keys_while_typing_starts_nothing() {
        // Typing "USA": the keys overlap, then one lifts before the timer runs out.
        let (hold, change) = hold_after_keys(Hold::Idle, true);
        assert_eq!((hold, change), (Hold::Arming, None));
        let (hold, change) = hold_after_keys(hold, false);
        assert_eq!((hold, change), (Hold::Idle, None));
        // A late timer finds nothing to do.
        assert_eq!(hold_after_arm_timer(hold, false), (Hold::Idle, None));
    }

    #[test]
    fn a_timer_racing_a_release_does_not_start_a_recording() {
        assert_eq!(
            hold_after_arm_timer(Hold::Arming, false),
            (Hold::Idle, None)
        );
    }

    #[test]
    fn a_real_hold_starts_after_the_minimum_and_ends_on_release() {
        let (hold, _) = hold_after_keys(Hold::Idle, true);
        // Key repeat while waiting changes nothing.
        assert_eq!(hold_after_keys(hold, true), (Hold::Arming, None));
        let (hold, change) = hold_after_arm_timer(hold, true);
        assert_eq!((hold, change), (Hold::Held, Some(HoldChange::Pressed)));
        assert_eq!(
            hold_after_keys(hold, true),
            (Hold::Held, None),
            "key repeat"
        );
        assert_eq!(
            hold_after_keys(hold, false),
            (Hold::Idle, Some(HoldChange::Released))
        );
    }

    #[test]
    fn the_minimum_hold_is_longer_than_a_typing_overlap() {
        assert!((150..=300).contains(&MIN_HOLD_MS));
    }

    /// A failed start reports why and leaves the monitor able to try again.
    #[cfg(windows)]
    #[test]
    fn a_failed_start_reports_why_and_can_be_retried() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        use std::time::Duration;
        static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
        fn failing(
            _handler: &native::SharedHandler,
            _is_down: native::IsDown,
        ) -> windows::core::Result<()> {
            ATTEMPTS.fetch_add(1, Ordering::SeqCst);
            Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x8000_4005_u32 as i32,
            )))
        }
        for attempt in 1..=2 {
            let (tx, rx) = mpsc::channel();
            native::set_enabled_with(
                true,
                || Box::new(move |change| tx.send(change).unwrap()),
                |_| false,
                failing,
            );
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                HoldChange::Unavailable(reason) => assert!(reason.contains("unavailable")),
                other => panic!("expected a reason, got {other:?}"),
            }
            assert_eq!(ATTEMPTS.load(Ordering::SeqCst), attempt, "tried again");
        }
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
            native::run_for_test,
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            HoldChange::Available
        );

        // A tap shorter than the minimum hold reports nothing.
        left_shift(KEYBD_EVENT_FLAGS(0));
        std::thread::sleep(Duration::from_millis(40));
        left_shift(KEYEVENTF_KEYUP);
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());

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

        native::set_enabled_with(
            false,
            || unreachable!("already started"),
            |_| true,
            native::run_for_test,
        );
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
