//! Clipboard paste delivery. Policy adapted from Hex (MIT); see THIRD_PARTY_NOTICES.md.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use zeroize::Zeroizing;

#[derive(Debug)]
pub(super) struct InsertionFailure {
    pub message: String,
    pub may_have_inserted: bool,
}
impl From<String> for InsertionFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            may_have_inserted: false,
        }
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Dictation cancelled".into())
    } else {
        Ok(())
    }
}

fn wait_for_modifiers(
    cancel: &AtomicBool,
    mut held: impl FnMut() -> bool,
    mut wait: impl FnMut(),
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    check_cancel(cancel)?;
    while held() {
        check_cancel(cancel)?;
        if std::time::Instant::now() > deadline {
            return Err("Release shortcut keys before inserting.".into());
        }
        wait();
    }
    check_cancel(cancel)
}

#[derive(Clone, Debug)]
pub(super) struct ForegroundTarget {
    #[cfg(windows)]
    window: isize,
    #[cfg(windows)]
    focus: isize,
    #[cfg(windows)]
    process_id: u32,
}

#[cfg(windows)]
use windows::Win32::{
    Foundation::HWND,
    System::Threading::GetCurrentProcessId,
    UI::{
        Input::KeyboardAndMouse::{SendInput, INPUT, VK_CONTROL, VK_V},
        WindowsAndMessaging::{
            GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetWindowLongPtrW,
            GetWindowThreadProcessId, IsIconic, GUITHREADINFO, GWL_STYLE,
        },
    },
};

#[cfg(windows)]
pub(super) fn capture_foreground() -> Option<ForegroundTarget> {
    unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return None;
        }
        let mut process_id = 0;
        let thread_id = GetWindowThreadProcessId(window, Some(&mut process_id));
        if !paste_allowed(thread_id != 0, process_id == GetCurrentProcessId(), false) {
            return None;
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let _ = GetGUIThreadInfo(thread_id, &mut info);
        Some(ForegroundTarget {
            window: window.0 as isize,
            focus: info.hwndFocus.0 as isize,
            process_id,
        })
    }
}

#[cfg(not(windows))]
pub(super) fn capture_foreground() -> Option<ForegroundTarget> {
    None
}

#[cfg(windows)]
pub(super) fn verify_foreground_target(target: &ForegroundTarget) -> Result<(), String> {
    unsafe {
        let window = HWND(target.window as _);
        if GetForegroundWindow() != window || IsIconic(window).as_bool() {
            return Err("The original app is no longer the foreground target. Automatic insertion was skipped.".into());
        }
        let mut process_id = 0;
        let thread_id = GetWindowThreadProcessId(window, Some(&mut process_id));
        if thread_id == 0 || process_id != target.process_id || process_id == GetCurrentProcessId()
        {
            return Err("The original input target is no longer available. Automatic insertion was skipped.".into());
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(thread_id, &mut info).is_err()
            || info.hwndFocus.0 as isize != target.focus
        {
            return Err("Focus moved away from the original input control. Automatic insertion was skipped.".into());
        }
        Ok(())
    }
}

#[cfg(not(windows))]
pub(super) fn verify_foreground_target(_: &ForegroundTarget) -> Result<(), String> {
    Err("Automatic text insertion is supported only on Windows.".into())
}

#[cfg(windows)]
pub(super) fn is_editable_target(target: &ForegroundTarget) -> Result<(), String> {
    unsafe {
        let focus = HWND(target.focus as _);
        let mut class_name = [0u16; 256];
        let length = GetClassNameW(focus, &mut class_name);
        if length <= 0 {
            return if focused_password() {
                Err("Password controls are excluded from dictation.".into())
            } else {
                Ok(())
            };
        }
        let class_name = String::from_utf16_lossy(&class_name[..length as usize]);
        let style = GetWindowLongPtrW(focus, GWL_STYLE) as u32;
        let recognized = class_name == "Edit" || class_name.starts_with("RichEdit");
        const ES_PASSWORD: u32 = 0x0020;
        if !paste_allowed(
            true,
            false,
            recognized && style & ES_PASSWORD != 0 || focused_password(),
        ) {
            return Err("Password controls are excluded from dictation.".into());
        }
        Ok(())
    }
}

#[cfg(windows)]
fn focused_password() -> bool {
    use windows::Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
        UI::Accessibility::{CUIAutomation, IUIAutomation},
    };
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let result = (|| -> windows::core::Result<bool> {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            Ok(automation
                .GetFocusedElement()?
                .CurrentIsPassword()?
                .as_bool())
        })()
        .unwrap_or(false);
        if initialized {
            CoUninitialize();
        }
        result
    }
}
#[cfg(not(windows))]
pub(super) fn is_editable_target(_: &ForegroundTarget) -> Result<(), String> {
    Err("Automatic text insertion is supported only on Windows.".into())
}

#[cfg(windows)]
mod clipboard {
    use super::*;
    use windows::Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL},
        System::{DataExchange::*, Memory::*},
    };
    type Snapshot = Vec<(u32, Zeroizing<Vec<u8>>)>;
    #[derive(Default)]
    struct Restore {
        generation: u64,
        sequence: Option<u32>,
        original: Option<Snapshot>,
    }
    impl Restore {
        fn register(&mut self, previous: Option<Snapshot>, before: u32, written: u32) -> u64 {
            self.generation = self.generation.wrapping_add(1);
            if self.sequence != Some(before) {
                self.original = previous;
            }
            self.sequence = Some(written);
            self.generation
        }
    }
    static RESTORE: Mutex<Restore> = Mutex::new(Restore {
        generation: 0,
        sequence: None,
        original: None,
    });
    struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }
    fn open(owner: isize) -> Result<Open, String> {
        unsafe {
            OpenClipboard(HWND(owner as _)).map_err(|e| e.to_string())?;
        }
        Ok(Open)
    }
    unsafe fn snapshot() -> Option<Snapshot> {
        let mut result = Vec::new();
        let mut format = 0;
        let mut total = 0usize;
        loop {
            format = EnumClipboardFormats(format);
            if format == 0 {
                return Some(result);
            }
            // Windows synthesizes bitmap/palette handles alongside DIB bytes.
            // The DIB retains the pixels and palette without copying GDI handles.
            if matches!(format, 2 | 9)
                && (IsClipboardFormatAvailable(8).is_ok() || IsClipboardFormatAvailable(17).is_ok())
            {
                continue;
            }
            // These formats contain GDI/owner handles, not transferable global bytes.
            if matches!(format, 2 | 3 | 9 | 14 | 0x80..=0x8e | 0x300..=0x3ff) {
                return None;
            }
            let handle = GetClipboardData(format).ok()?;
            let memory = HGLOBAL(handle.0);
            let size = GlobalSize(memory);
            if size == 0 || size > 64 * 1024 * 1024 || result.len() >= 128 {
                return None;
            }
            total = total.checked_add(size)?;
            if total > 256 * 1024 * 1024 {
                return None;
            }
            let ptr = GlobalLock(memory) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let bytes = Zeroizing::new(std::slice::from_raw_parts(ptr, size).to_vec());
            let _ = GlobalUnlock(memory);
            result.push((format, bytes));
        }
    }
    unsafe fn set(format: u32, bytes: &[u8]) -> Result<(), String> {
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| e.to_string())?;
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(mem);
            return Err("Clipboard allocation failed".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(mem);
        if let Err(e) = SetClipboardData(format, HANDLE(mem.0)) {
            let _ = GlobalFree(mem);
            return Err(e.to_string());
        }
        Ok(())
    }
    unsafe fn mark() -> Result<(), String> {
        for name in [
            "ExcludeClipboardContentFromMonitorProcessing",
            "CanIncludeInClipboardHistory",
            "CanUploadToCloudClipboard",
        ] {
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let id = RegisterClipboardFormatW(windows::core::PCWSTR(wide.as_ptr()));
            if id == 0 {
                return Err("Clipboard privacy format unavailable".into());
            }
            set(id, &0u32.to_ne_bytes())?;
        }
        Ok(())
    }
    pub(super) fn write(
        text: &str,
        owner: isize,
        cancel: &AtomicBool,
        gate: &Mutex<()>,
    ) -> Result<bool, InsertionFailure> {
        check_cancel(cancel)?;
        let _gate = gate
            .lock()
            .map_err(|_| "Insertion unavailable".to_owned())?;
        check_cancel(cancel)?;
        let mut state = RESTORE
            .lock()
            .map_err(|_| "Clipboard unavailable".to_owned())?;
        let mut history = crate::clipboard_history::DICTATION_CLIPBOARD
            .lock()
            .map_err(|_| "Clipboard unavailable".to_owned())?;
        let opened = open(owner)?;
        let before = unsafe { GetClipboardSequenceNumber() };
        let previous = unsafe { snapshot() };

        check_cancel(cancel)?;
        unsafe {
            EmptyClipboard().map_err(|e| e.to_string())?;
            mark()?;
            let units = Zeroizing::new(text.encode_utf16().chain(Some(0)).collect::<Vec<_>>());
            let bytes = std::slice::from_raw_parts(units.as_ptr() as *const u8, units.len() * 2);
            set(13, bytes)?;
        }
        let sequence = unsafe { GetClipboardSequenceNumber() };
        drop(opened);
        *history = sequence;
        let generation = state.register(previous, before, sequence);
        drop(history);
        drop(_gate);
        drop(state);
        let remote = remote_desktop();
        if remote {
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        let wait = {
            wait_for_modifiers(cancel, modifiers_held, || {
                std::thread::sleep(std::time::Duration::from_millis(10))
            })
        };
        // Accessibility providers can stall. Never hold the cancellation gate while querying them.
        let eligible = if wait.is_ok() && !cancel.load(Ordering::Acquire) {
            capture_foreground().filter(|target| is_editable_target(target).is_ok())
        } else {
            None
        };
        let _gate = gate
            .lock()
            .map_err(|_| "Insertion unavailable".to_owned())?;
        if unsafe { GetClipboardSequenceNumber() } != sequence {
            return Err("The clipboard changed before paste. Review the transcript."
                .to_owned()
                .into());
        }
        let allowed = !modifiers_held()
            && !cancel.load(Ordering::Acquire)
            && eligible
                .as_ref()
                .is_some_and(|target| verify_foreground_target(target).is_ok());
        let mut uncertain = false;
        if allowed {
            let inputs = [
                key(VK_CONTROL, false),
                key(VK_V, false),
                key(VK_V, true),
                key(VK_CONTROL, true),
            ];
            let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
            if sent != 4 {
                unsafe {
                    SendInput(
                        &[key(VK_V, true), key(VK_CONTROL, true)],
                        std::mem::size_of::<INPUT>() as i32,
                    );
                }
                uncertain = true;
            }
        }
        let cancelled = cancel.load(Ordering::Acquire);
        let mut state = RESTORE.lock().unwrap_or_else(|p| p.into_inner());
        if !allowed && !cancelled {
            state.original = None;
            state.sequence = None;
        }
        drop(state);
        if allowed || cancelled {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(if !allowed {
                    0
                } else if remote {
                    3000
                } else {
                    500
                }));
                let mut state = RESTORE.lock().unwrap_or_else(|p| p.into_inner());
                if state.generation != generation {
                    return;
                }
                let mut history = crate::clipboard_history::DICTATION_CLIPBOARD
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                let Ok(opened) = open(owner) else {
                    return;
                };
                if !restore_allowed(state.generation, generation, state.sequence, unsafe {
                    GetClipboardSequenceNumber()
                }) {
                    state.original = None;
                    state.sequence = None;
                    return;
                }
                if let Some(snapshot) = state.original.take() {
                    unsafe {
                        if EmptyClipboard().is_ok() {
                            for (format, bytes) in snapshot {
                                if set(format, &bytes).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    *history = unsafe { GetClipboardSequenceNumber() };
                    drop(opened);
                }
                state.sequence = None;
            });
        }
        check_cancel(cancel)?;
        if uncertain {
            return Err(InsertionFailure {
                message:
                    "Windows did not accept the entire paste shortcut. Check the target manually."
                        .into(),
                may_have_inserted: true,
            });
        }
        Ok(allowed)
    }
    fn modifiers_held() -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            .any(|k| unsafe { GetAsyncKeyState(k.0 as i32) } < 0)
    }
    fn remote_desktop() -> bool {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
            let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut buffer = [0u16; 32768];
            let mut length = buffer.len() as u32;
            let result = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buffer.as_mut_ptr()),
                &mut length,
            );
            let _ = CloseHandle(process);
            result.is_ok()
                && std::path::Path::new(&String::from_utf16_lossy(&buffer[..length as usize]))
                    .file_stem()
                    .is_some_and(|stem| matches!(stem.to_str(), Some("mstsc" | "msrdc")))
        }
    }
    fn key(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY, up: bool) -> INPUT {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        unsafe {
            let thread = windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
                windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow(),
                None,
            );
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: MapVirtualKeyExW(
                            vk.0 as u32,
                            MAPVK_VK_TO_VSC,
                            GetKeyboardLayout(thread),
                        ) as u16,
                        dwFlags: if up {
                            KEYEVENTF_KEYUP
                        } else {
                            Default::default()
                        },
                        ..Default::default()
                    },
                },
            }
        }
    }
    #[cfg(test)]
    mod restore_tests {
        use super::*;
        #[test]
        fn rapid_pastes_preserve_original_and_supersede_old_restore() {
            let mut state = Restore::default();
            let first = state.register(Some(vec![(13, Zeroizing::new(vec![1]))]), 10, 11);
            let second = state.register(Some(vec![(13, Zeroizing::new(vec![2]))]), 11, 12);
            assert_ne!(first, second);
            assert_eq!(state.original.as_ref().unwrap()[0].1.as_slice(), &[1]);
            assert!(!restore_allowed(
                state.generation,
                first,
                state.sequence,
                12
            ));
            assert!(restore_allowed(
                state.generation,
                second,
                state.sequence,
                12
            ));
            assert!(!restore_allowed(
                state.generation,
                second,
                state.sequence,
                13
            ));
        }
        #[test]
        fn unsupported_snapshot_does_not_restore_old_contents() {
            let mut state = Restore::default();
            state.register(Some(vec![]), 1, 2);
            state.register(None, 5, 6);
            assert!(state.original.is_none());
        }
    }
}
#[cfg(any(windows, test))]
fn restore_allowed(current: u64, requested: u64, expected: Option<u32>, actual: u32) -> bool {
    current == requested && expected == Some(actual)
}
#[cfg(windows)]
pub(super) fn paste(
    text: &str,
    cancel: &AtomicBool,
    gate: &Mutex<()>,
    owner: isize,
) -> Result<bool, InsertionFailure> {
    clipboard::write(text, owner, cancel, gate)
}
#[cfg(not(windows))]
pub(super) fn paste(
    _: &str,
    _: &AtomicBool,
    _: &Mutex<()>,
    _: isize,
) -> Result<bool, InsertionFailure> {
    Err("Windows required".to_owned().into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_clipboard_and_newer_paste_prevent_restore() {
        assert!(restore_allowed(2, 2, Some(4), 4));
        assert!(!restore_allowed(2, 2, Some(4), 5));
        assert!(!restore_allowed(3, 2, Some(4), 4));
    }
    #[test]
    fn cancel_before_paste() {
        assert!(check_cancel(&AtomicBool::new(true)).is_err());
    }
    #[test]
    fn cancel_during_modifier_wait() {
        let c = AtomicBool::new(false);
        assert!(wait_for_modifiers(
            &c,
            || !c.load(Ordering::Acquire),
            || c.store(true, Ordering::Release)
        )
        .is_err());
    }
}

fn paste_allowed(foreground: bool, own_process: bool, password: bool) -> bool {
    foreground && !own_process && !password
}
#[cfg(test)]
mod policy_tests {
    #[test]
    fn own_missing_and_password_targets_copy() {
        assert!(!super::paste_allowed(true, true, false));
        assert!(!super::paste_allowed(false, false, false));
        assert!(!super::paste_allowed(true, false, true));
        assert!(super::paste_allowed(true, false, false));
    }
}
