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
        Foundation::{GetLastError, GlobalFree, SetLastError, HANDLE, HGLOBAL, WIN32_ERROR},
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
    static RESTORE_SHUTDOWN: AtomicBool = AtomicBool::new(false);
    struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }
    /// Another app may hold the clipboard for a moment, so one refusal is not final.
    fn open(owner: isize) -> Result<Open, String> {
        let mut attempt = 0;
        loop {
            match unsafe { OpenClipboard(HWND(owner as _)) } {
                Ok(()) => return Ok(Open),
                Err(error) if attempt == 9 => return Err(error.to_string()),
                Err(_) => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    }
    unsafe fn snapshot() -> Result<Option<Snapshot>, String> {
        let mut result = Vec::new();
        let mut format = 0;
        let mut total = 0usize;
        loop {
            SetLastError(WIN32_ERROR(0));
            format = EnumClipboardFormats(format);
            if format == 0 {
                return if GetLastError().0 == 0 {
                    Ok(Some(result))
                } else {
                    Err("Could not enumerate clipboard formats".into())
                };
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
                return Ok(None);
            }
            let handle = GetClipboardData(format)
                .map_err(|error| format!("Could not read clipboard format {format}: {error}"))?;
            let memory = HGLOBAL(handle.0);
            let size = GlobalSize(memory);
            if size == 0 || size > 64 * 1024 * 1024 || result.len() >= 128 {
                return Ok(None);
            }
            total = total
                .checked_add(size)
                .ok_or_else(|| "Clipboard snapshot size overflow".to_owned())?;
            if total > 256 * 1024 * 1024 {
                return Ok(None);
            }
            let ptr = GlobalLock(memory) as *const u8;
            if ptr.is_null() {
                return Err(format!("Could not lock clipboard format {format}"));
            }
            let bytes = Zeroizing::new(std::slice::from_raw_parts(ptr, size).to_vec());
            let _ = GlobalUnlock(memory);
            result.push((format, bytes));
        }
    }
    unsafe fn alloc(bytes: &[u8]) -> Result<HGLOBAL, String> {
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| e.to_string())?;
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(mem);
            return Err("Clipboard allocation failed".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(mem);
        Ok(mem)
    }
    /// The formats that keep a write out of Windows clipboard history and cloud sync.
    unsafe fn privacy_formats() -> Result<[u32; 3], String> {
        let mut ids = [0; 3];
        for (id, name) in ids.iter_mut().zip([
            "ExcludeClipboardContentFromMonitorProcessing",
            "CanIncludeInClipboardHistory",
            "CanUploadToCloudClipboard",
        ]) {
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            *id = RegisterClipboardFormatW(windows::core::PCWSTR(wide.as_ptr()));
            if *id == 0 {
                return Err("Clipboard privacy format unavailable".into());
            }
        }
        Ok(ids)
    }
    /// A failed replacement, and whether the clipboard had already been emptied.
    struct ReplaceError {
        cleared: bool,
        message: String,
    }
    /// Replaces the open clipboard with `items` plus the privacy formats. Every block is
    /// allocated before the clipboard is emptied, so running out of memory changes nothing.
    unsafe fn replace(items: &[(u32, &[u8])]) -> Result<(), ReplaceError> {
        let mut prepared = Vec::with_capacity(items.len() + 3);
        let free = |prepared: Vec<(u32, HGLOBAL)>| {
            for (_, mem) in prepared {
                let _ = GlobalFree(mem);
            }
        };
        let allocated = (|| {
            let privacy = privacy_formats()?;
            for id in privacy {
                prepared.push((id, alloc(&0u32.to_ne_bytes())?));
            }
            for (format, bytes) in items {
                if !privacy.contains(format) {
                    prepared.push((*format, alloc(bytes)?));
                }
            }
            Ok(())
        })();
        if let Err(message) = allocated {
            free(prepared);
            return Err(ReplaceError {
                cleared: false,
                message,
            });
        }
        if let Err(error) = EmptyClipboard() {
            free(prepared);
            return Err(ReplaceError {
                cleared: false,
                message: error.to_string(),
            });
        }
        let mut failure = None;
        for (format, mem) in prepared {
            // The clipboard owns a block only once it accepts it.
            if failure.is_some() {
                let _ = GlobalFree(mem);
            } else if let Err(error) = SetClipboardData(format, HANDLE(mem.0)) {
                let _ = GlobalFree(mem);
                failure = Some(error.to_string());
            }
        }
        match failure {
            None => Ok(()),
            Some(message) => Err(ReplaceError {
                cleared: true,
                message,
            }),
        }
    }
    fn items(snapshot: &Snapshot) -> Vec<(u32, &[u8])> {
        snapshot
            .iter()
            .map(|(format, bytes)| (*format, bytes.as_slice()))
            .collect()
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
        let snapshot = unsafe { snapshot() };
        check_cancel(cancel)?;
        let (previous, write_result): (Snapshot, Result<(), ReplaceError>) =
            with_complete_snapshot(snapshot, |previous| unsafe {
                let units = Zeroizing::new(text.encode_utf16().chain(Some(0)).collect::<Vec<_>>());
                let bytes =
                    std::slice::from_raw_parts(units.as_ptr() as *const u8, units.len() * 2);
                (previous, replace(&[(13, bytes)]))
            })?;
        unsafe {
            if let Err(error) = write_result {
                if error.cleared {
                    // Put back what was there; a half-written transcript must not stay.
                    let restored = replace(&items(&previous)).is_ok();
                    if !restored {
                        let _ = EmptyClipboard();
                    }
                    drop(opened);
                    *history = GetClipboardSequenceNumber();
                }
                return Err(error.message.into());
            }
        }
        // Closing the clipboard bumps the sequence number again (Windows adds the synthesized
        // text formats then), so it is read only once the clipboard is closed.
        drop(opened);
        let sequence = unsafe { GetClipboardSequenceNumber() };
        *history = sequence;
        let generation = state.register(Some(previous), before, sequence);
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
                // Keep the zeroizing snapshot until restoration succeeds, another app
                // writes, a newer paste supersedes it, or app shutdown is requested.
                let outcome = retry_restore_until(
                    || restore_once(owner, generation),
                    |delay| {
                        if RESTORE_SHUTDOWN.load(Ordering::Acquire) {
                            return false;
                        }
                        std::thread::sleep(delay);
                        !RESTORE_SHUTDOWN.load(Ordering::Acquire)
                    },
                );
                if outcome == RestoreAttempt::Shutdown {
                    eprintln!("Clipboard restoration stopped during application shutdown");
                    let mut state = RESTORE.lock().unwrap_or_else(|p| p.into_inner());
                    if state.generation == generation {
                        state.original = None;
                        state.sequence = None;
                    }
                }
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
    pub(super) fn shutdown_restore_workers() {
        RESTORE_SHUTDOWN.store(true, Ordering::Release);
        let mut state = RESTORE.lock().unwrap_or_else(|p| p.into_inner());
        if state.original.take().is_some() {
            eprintln!("Clipboard restoration abandoned during application shutdown");
        }
        state.sequence = None;
        state.generation = state.generation.wrapping_add(1);
    }
    /// One attempt to restore, distinguishing contention from terminal outcomes.
    fn restore_once(owner: isize, generation: u64) -> RestoreAttempt {
        let mut state = RESTORE.lock().unwrap_or_else(|p| p.into_inner());
        if state.generation != generation {
            return RestoreAttempt::Superseded;
        }
        let mut history = crate::clipboard_history::DICTATION_CLIPBOARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Ok(opened) = open(owner) else {
            return RestoreAttempt::Retry;
        };
        if !restore_allowed(state.generation, generation, state.sequence, unsafe {
            GetClipboardSequenceNumber()
        }) {
            state.original = None;
            state.sequence = None;
            return RestoreAttempt::ClipboardChanged;
        }
        let Some(snapshot) = state.original.as_ref() else {
            state.sequence = None;
            return RestoreAttempt::Restored;
        };
        let result = unsafe { replace(&items(snapshot)) };
        // Tuck's own history skips this write, and a retry still recognises the clipboard as ours.
        // Read after closing: closing bumps the sequence number.
        drop(opened);
        let sequence = unsafe { GetClipboardSequenceNumber() };
        *history = sequence;
        match result {
            Ok(()) => {
                state.original = None;
                state.sequence = None;
                RestoreAttempt::Restored
            }
            Err(_) => {
                state.sequence = Some(sequence);
                RestoreAttempt::Retry
            }
        }
    }
    /// Leaves `text` on the clipboard for the owner to paste, with no restore. Used when an
    /// automatic paste could not happen, so the words are never lost.
    pub(super) fn copy(text: &str, owner: isize, gate: &Mutex<()>) -> Result<(), String> {
        let _gate = gate
            .lock()
            .map_err(|_| "Insertion unavailable".to_owned())?;
        let mut state = RESTORE.lock().unwrap_or_else(|p| p.into_inner());
        let mut history = crate::clipboard_history::DICTATION_CLIPBOARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // A pending restore would put the old clipboard back over these words.
        state.generation = state.generation.wrapping_add(1);
        state.original = None;
        state.sequence = None;
        let opened = open(owner)?;
        let units = Zeroizing::new(text.encode_utf16().chain(Some(0)).collect::<Vec<_>>());
        let bytes =
            unsafe { std::slice::from_raw_parts(units.as_ptr() as *const u8, units.len() * 2) };
        let result = unsafe { replace(&[(13, bytes)]) };
        drop(opened);
        *history = unsafe { GetClipboardSequenceNumber() };
        result.map_err(|error| error.message)
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
        #[test]
        fn incomplete_snapshot_never_replaces_clipboard_and_empty_is_valid() {
            let mut replaced = false;
            let snapshots: [Result<Option<Snapshot>, String>; 3] = [
                Ok(None),
                Err("enumeration failed".to_owned()),
                Err("clipboard read failed".to_owned()),
            ];
            for snapshot in snapshots {
                let result = with_complete_snapshot(snapshot, |_| replaced = true);
                assert!(result.is_err());
                assert!(
                    !replaced,
                    "incomplete snapshot must fail before replacement"
                );
            }
            let empty: Snapshot = Vec::new();
            with_complete_snapshot(Ok(Some(empty)), |snapshot| {
                replaced = true;
                assert!(snapshot.is_empty());
            })
            .unwrap();
            assert!(replaced);
        }
        #[test]
        fn restoration_retry_backoff_is_capped() {
            assert_eq!(restore_retry_delay(0).as_millis(), 25);
            assert_eq!(restore_retry_delay(1).as_millis(), 50);
            assert_eq!(restore_retry_delay(20).as_millis(), 800);
        }
        #[test]
        fn restoration_retries_until_success_and_stops_on_sequence_change() {
            let mut attempts = [
                RestoreAttempt::Retry,
                RestoreAttempt::Retry,
                RestoreAttempt::Restored,
            ]
            .into_iter();
            let mut waits = Vec::new();
            let outcome = retry_restore_until(
                || attempts.next().unwrap(),
                |delay| {
                    waits.push(delay);
                    true
                },
            );
            assert_eq!(outcome, RestoreAttempt::Restored);
            assert_eq!(waits, [restore_retry_delay(0), restore_retry_delay(1)]);

            let mut waits = 0;
            assert_eq!(
                retry_restore_until(
                    || RestoreAttempt::ClipboardChanged,
                    |_| {
                        waits += 1;
                        true
                    }
                ),
                RestoreAttempt::ClipboardChanged
            );
            assert_eq!(waits, 0, "resolved sequence changes are not retried");
            assert_eq!(
                retry_restore_until(|| RestoreAttempt::Retry, |_| false),
                RestoreAttempt::Shutdown
            );
        }
    }
}
#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RestoreAttempt {
    Retry,
    Restored,
    ClipboardChanged,
    Superseded,
    Shutdown,
}
#[cfg(any(windows, test))]
fn retry_restore_until(
    mut attempt: impl FnMut() -> RestoreAttempt,
    mut wait: impl FnMut(std::time::Duration) -> bool,
) -> RestoreAttempt {
    let mut retries = 0u32;
    loop {
        match attempt() {
            RestoreAttempt::Retry => {
                if !wait(restore_retry_delay(retries)) {
                    return RestoreAttempt::Shutdown;
                }
                retries = retries.saturating_add(1);
            }
            done => return done,
        }
    }
}
fn restore_retry_delay(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_millis(25u64.saturating_mul(1u64 << attempt.min(5)).min(800))
}
fn with_complete_snapshot<T, R>(
    snapshot: Result<Option<T>, String>,
    replace: impl FnOnce(T) -> R,
) -> Result<R, InsertionFailure> {
    match snapshot {
        Ok(Some(snapshot)) => Ok(replace(snapshot)),
        Ok(None) => Err(
            "Clipboard formats cannot be safely preserved. Review the transcript."
                .to_owned()
                .into(),
        ),
        Err(error) => Err(format!(
            "Could not safely preserve the clipboard: {error}. Review the transcript."
        )
        .into()),
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
#[cfg(windows)]
pub(super) fn shutdown_restore_workers() {
    clipboard::shutdown_restore_workers();
}
#[cfg(not(windows))]
pub(super) fn shutdown_restore_workers() {}
#[cfg(windows)]
pub(super) fn copy(text: &str, gate: &Mutex<()>, owner: isize) -> Result<(), String> {
    clipboard::copy(text, owner, gate)
}
#[cfg(not(windows))]
pub(super) fn copy(_: &str, _: &Mutex<()>, _: isize) -> Result<(), String> {
    Err("Windows required".to_owned())
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
