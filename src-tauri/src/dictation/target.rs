//! Conservative Windows target capture and Unicode insertion.
//! Clipboard is never touched: text is sent with Unicode keyboard input, so
//! concurrent clipboard changes are preserved without a restore race.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use zeroize::{Zeroize, Zeroizing};

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

fn guarded_send(
    cancel: &AtomicBool,
    gate: &Mutex<()>,
    send: impl FnOnce() -> usize,
) -> Result<usize, String> {
    let _guard = gate.lock().map_err(|_| "Insertion unavailable")?;
    check_cancel(cancel)?;
    Ok(send())
}

pub(super) fn send_batches(
    units: &[u16],
    mut verify: impl FnMut() -> Result<(), String>,
    mut send: impl FnMut(&[u16]) -> usize,
) -> Result<(), InsertionFailure> {
    let mut accepted = false;
    for chunk in units.chunks(256) {
        verify().map_err(|message| InsertionFailure {
            message: if accepted { format!("{message} Some text may already be inserted. Reconcile the target manually; automatic retry is disabled.") } else { message },
            may_have_inserted: accepted,
        })?;
        let inserted = send(chunk);
        if inserted != chunk.len() * 2 {
            return Err(InsertionFailure { message: if accepted || inserted > 0 { "Windows accepted only part of the insertion. Check the target text manually; automatic retry is disabled." } else { "Windows did not accept the insertion. Check the input target and permissions." }.into(), may_have_inserted: accepted || inserted > 0 });
        }
        accepted = true;
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) struct ForegroundTarget {
    #[cfg(windows)]
    window: isize,
    #[cfg(windows)]
    focus: isize,
    #[cfg(windows)]
    process_id: u32,
    #[cfg(windows)]
    runtime_id: Option<Vec<i32>>,
}

#[cfg(windows)]
use windows::Win32::{
    Foundation::HWND,
    System::Threading::GetCurrentProcessId,
    UI::{
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            KEYEVENTF_UNICODE,
        },
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
        if thread_id == 0 || process_id == GetCurrentProcessId() {
            return None;
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(thread_id, &mut info).is_err() || info.hwndFocus.0.is_null() {
            return None;
        }
        Some(ForegroundTarget {
            window: window.0 as isize,
            focus: info.hwndFocus.0 as isize,
            process_id,
            runtime_id: focused_edit_identity().ok(),
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
        if let Some(expected) = &target.runtime_id {
            if focused_edit_identity().as_ref() != Ok(expected) {
                return Err(
                    "The focused text field changed. Automatic insertion was skipped.".into(),
                );
            }
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
            return Err("The focused input control could not be identified. Automatic insertion was skipped.".into());
        }
        let class_name = String::from_utf16_lossy(&class_name[..length as usize]);
        let style = GetWindowLongPtrW(focus, GWL_STYLE) as u32;
        let recognized = class_name == "Edit" || class_name.starts_with("RichEdit");
        const ES_READONLY: u32 = 0x0800;
        const ES_PASSWORD: u32 = 0x0020;
        if recognized && (style & (ES_READONLY | ES_PASSWORD)) != 0 {
            return Err("Read-only and password controls are excluded from dictation.".into());
        }
        if !recognized && target.runtime_id.is_none() {
            return Err("This app's focused control is not a supported editable Windows text field. Review the transcript and place focus in a standard edit field before inserting.".into());
        }
        Ok(())
    }
}

/// UI Automation identifies browser/Electron input fields without retaining their contents.
#[cfg(windows)]
fn focused_edit_identity() -> Result<Vec<i32>, String> {
    use windows::Win32::{
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_MULTITHREADED,
            },
            Ole::{
                SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound,
                SafeArrayUnaccessData,
            },
        },
        UI::Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationTextPattern, IUIAutomationValuePattern,
            UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_IsReadOnlyAttributeId,
            UIA_TextPatternId, UIA_ValuePatternId,
        },
    };
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        // RPC_E_CHANGED_MODE means the calling UI thread already has an STA; it remains usable.
        let result = (|| {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                    .map_err(|_| "Windows accessibility is unavailable")?;
            let focused = automation
                .GetFocusedElement()
                .map_err(|_| "No accessible focused field")?;
            if focused.CurrentIsPassword().map_or(true, |v| v.as_bool())
                || !focused.CurrentIsEnabled().is_ok_and(|v| v.as_bool())
                || !focused.CurrentHasKeyboardFocus().is_ok_and(|v| v.as_bool())
            {
                return Err("The focused field cannot accept dictation".to_owned());
            }
            let value: Option<IUIAutomationValuePattern> =
                focused.GetCurrentPatternAs(UIA_ValuePatternId).ok();
            let editable = value
                .as_ref()
                .is_some_and(|v| v.CurrentIsReadOnly().is_ok_and(|r| !r.as_bool()));
            let text_editable = focused
                .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                .ok()
                .and_then(|p| p.DocumentRange().ok())
                .and_then(|r| r.GetAttributeValue(UIA_IsReadOnlyAttributeId).ok())
                .is_some_and(|v| bool::try_from(&v).ok() == Some(false));
            let kind = focused.CurrentControlType().ok();
            if !(editable || text_editable)
                || !(kind == Some(UIA_EditControlTypeId) || kind == Some(UIA_DocumentControlTypeId))
            {
                return Err("The focused control is not an editable text input".into());
            }
            let array = focused
                .GetRuntimeId()
                .map_err(|_| "Could not identify input field")?;
            if array.is_null() {
                return Err("Input identity unavailable".into());
            }
            let copy = (|| {
                let lower = SafeArrayGetLBound(array, 1).map_err(|_| "Invalid field identity")?;
                let upper = SafeArrayGetUBound(array, 1).map_err(|_| "Invalid field identity")?;
                let length = (upper as i64 - lower as i64 + 1).clamp(0, 1025) as usize;
                if length == 0 || length > 1024 {
                    return Err("Invalid field identity".to_owned());
                }
                let mut data = std::ptr::null_mut();
                SafeArrayAccessData(array, &mut data).map_err(|_| "Field identity unavailable")?;
                let ids = std::slice::from_raw_parts(data as *const i32, length).to_vec();
                let _ = SafeArrayUnaccessData(array);
                Ok(ids)
            })();
            let _ = SafeArrayDestroy(array);
            copy
        })();
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
pub(super) fn insert_unicode(
    target: &ForegroundTarget,
    text: &str,
    cancel: &AtomicBool,
    gate: &Mutex<()>,
) -> Result<(), InsertionFailure> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    check_cancel(cancel)?;
    wait_for_modifiers(
        cancel,
        || {
            [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
                .iter()
                .any(|key| unsafe { GetAsyncKeyState(key.0 as i32) } < 0)
        },
        || std::thread::sleep(std::time::Duration::from_millis(10)),
    )?;
    check_cancel(cancel)?;
    verify_foreground_target(target)?;
    check_cancel(cancel)?;
    is_editable_target(target)?;
    check_cancel(cancel)?;
    let units = Zeroizing::new(text.encode_utf16().collect::<Vec<u16>>());
    if units.len() > 32 * 1024 {
        return Err(String::from("Transcript is too long to insert safely").into());
    }
    let before = Zeroizing::new(focused_text().map_err(InsertionFailure::from)?);
    let selection = focused_selection(&before).ok();
    send_batches(
        &units,
        || {
            check_cancel(cancel)
                .and_then(|_| verify_foreground_target(target))
                .and_then(|_| is_editable_target(target))
                .and_then(|_| check_cancel(cancel))
        },
        |chunk| {
            let mut inputs = Vec::with_capacity(chunk.len() * 2);
            for unit in chunk {
                inputs.push(key_input(*unit, Default::default()));
                inputs.push(key_input(*unit, KEYEVENTF_KEYUP));
            }
            unsafe {
                let inserted = guarded_send(cancel, gate, || {
                    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) as usize
                })
                .unwrap_or(0);
                for input in &mut inputs {
                    input.Anonymous.ki.wScan.zeroize();
                }
                inserted
            }
        },
    )?;
    // Event acceptance is not application acceptance. Never retry after uncertain delivery.
    for _ in 0..20 {
        if check_cancel(cancel).is_err() || verify_foreground_target(target).is_err() {
            break;
        }
        if let Ok(after) = focused_text() {
            let after = Zeroizing::new(after);
            if confirms_replacement(&before, &after, text, selection) {
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Err(InsertionFailure { message: "Input was sent but the target did not confirm the text. Check it manually; automatic retry and copy are disabled.".into(), may_have_inserted: true })
}

fn confirms_insertion(before: &str, after: &str, text: &str) -> bool {
    !text.is_empty()
        && after.len() == before.len() + text.len()
        && after
            .match_indices(text)
            .any(|(i, _)| before.strip_prefix(&after[..i]) == Some(&after[i + text.len()..]))
}

fn confirms_replacement(
    before: &str,
    after: &str,
    text: &str,
    selection: Option<(usize, usize)>,
) -> bool {
    if let Some((start, end)) = selection {
        return !text.is_empty()
            && start <= end
            && before.is_char_boundary(start)
            && before.is_char_boundary(end)
            && end <= before.len()
            && after.len() == before.len() - (end - start) + text.len()
            && after
                .strip_prefix(&before[..start])
                .and_then(|s| s.strip_prefix(text))
                == Some(&before[end..]);
    }
    // Without a supported selection, only an exact additive change is confirmed.
    confirms_insertion(before, after, text)
}

#[cfg(windows)]
fn focused_selection(before: &str) -> Result<(usize, usize), String> {
    use windows::Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
        UI::Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationTextPattern, TextPatternRangeEndpoint_End,
            TextPatternRangeEndpoint_Start, UIA_TextPatternId,
        },
    };
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let result = (|| -> windows::core::Result<(usize, usize)> {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            let focused = automation.GetFocusedElement()?;
            let pattern: IUIAutomationTextPattern =
                focused.GetCurrentPatternAs(UIA_TextPatternId)?;
            let document = Zeroizing::new(pattern.DocumentRange()?.GetText(65537)?.to_string());
            if document.as_str() != before {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80004005u32 as i32,
                )));
            }
            let selections = pattern.GetSelection()?;
            if selections.Length()? != 1 {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80004005u32 as i32,
                )));
            }
            let selected = selections.GetElement(0)?;
            let prefix = pattern.DocumentRange()?;
            prefix.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &selected,
                TextPatternRangeEndpoint_Start,
            )?;
            let prefix = Zeroizing::new(prefix.GetText(65537)?.to_string());
            let selected = Zeroizing::new(selected.GetText(65537)?.to_string());
            if !before
                .strip_prefix(prefix.as_str())
                .is_some_and(|suffix| suffix.starts_with(selected.as_str()))
            {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80004005u32 as i32,
                )));
            }
            Ok((prefix.len(), prefix.len() + selected.len()))
        })()
        .map_err(|e| e.to_string())
        .and_then(|(start, end)| {
            if start <= end
                && end <= before.len()
                && before.is_char_boundary(start)
                && before.is_char_boundary(end)
            {
                Ok((start, end))
            } else {
                Err("Selection unavailable".into())
            }
        });
        if initialized {
            CoUninitialize();
        }
        result
    }
}

#[cfg(windows)]
fn focused_text() -> Result<String, String> {
    use windows::Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
        UI::Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationTextPattern, IUIAutomationValuePattern,
            UIA_TextPatternId, UIA_ValuePatternId,
        },
    };
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let result = (|| {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| e.to_string())?;
            let focused = automation.GetFocusedElement().map_err(|e| e.to_string())?;
            let value = if let Ok(p) =
                focused.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            {
                p.CurrentValue().map_err(|e| e.to_string())?.to_string()
            } else {
                focused
                    .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                    .and_then(|p| p.DocumentRange())
                    .and_then(|r| r.GetText(65537))
                    .map_err(|e| e.to_string())?
                    .to_string()
            };
            if value.len() > 65536 {
                return Err("Target is too large to verify safely".into());
            }
            Ok(value)
        })();
        if initialized {
            CoUninitialize();
        }
        result
    }
}

#[cfg(windows)]
fn key_input(
    unit: u16,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: Default::default(),
                wScan: unit,
                dwFlags: KEYEVENTF_UNICODE | flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(not(windows))]
pub(super) fn insert_unicode(
    _: &ForegroundTarget,
    _: &str,
    _: &AtomicBool,
    _: &Mutex<()>,
) -> Result<(), InsertionFailure> {
    Err(String::from("Automatic text insertion is supported only on Windows.").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_while_waiting_for_shift_stops_even_when_shift_is_released() {
        let cancel = AtomicBool::new(false);
        let held = std::cell::Cell::new(true);
        let result = wait_for_modifiers(
            &cancel,
            || held.get(),
            || {
                cancel.store(true, Ordering::Release);
                held.set(false);
            },
        );
        assert_eq!(result.unwrap_err(), "Dictation cancelled");
    }

    #[test]
    fn cancellation_during_validation_or_between_batches_never_sends_more() {
        let cancel = AtomicBool::new(false);
        let gate = Mutex::new(());
        let sent = std::cell::Cell::new(0);
        let result = send_batches(
            &[65; 600],
            || Ok(()),
            |chunk| {
                let n = guarded_send(&cancel, &gate, || {
                    sent.set(sent.get() + 1);
                    chunk.len() * 2
                })
                .unwrap_or(0);
                {
                    let _guard = gate.lock().unwrap();
                    cancel.store(true, Ordering::Release);
                }
                n
            },
        );
        assert!(result.unwrap_err().may_have_inserted);
        assert_eq!(sent.get(), 1);
        assert!(guarded_send(&cancel, &gate, || panic!("typed after cancel")).is_err());
        assert!(check_cancel(&cancel).is_err()); // also used during modifier-release wait
    }

    #[test]
    fn partial_batch_or_later_focus_failure_disables_whole_text_retry() {
        let partial = send_batches(&[65, 66], || Ok(()), |_| 1).unwrap_err();
        assert!(partial.may_have_inserted);
        let blocked = send_batches(&[65], || Ok(()), |_| 0).unwrap_err();
        assert!(!blocked.may_have_inserted);
        let mut checks = 0;
        let after_prefix = send_batches(
            &vec![65; 300],
            || {
                checks += 1;
                if checks == 1 {
                    Ok(())
                } else {
                    Err("Focus changed".into())
                }
            },
            |chunk| chunk.len() * 2,
        )
        .unwrap_err();
        assert!(after_prefix.may_have_inserted);
    }
}

#[cfg(windows)]
pub(super) fn copy_final(
    text: &str,
    owner: windows::Win32::Foundation::HWND,
) -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::{
            Foundation::{GlobalFree, HANDLE},
            System::{
                DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
                Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            },
        };
        let units = Zeroizing::new(
            text.encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>(),
        );
        let memory = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2)
            .map_err(|_| "Clipboard allocation failed")?;
        let ptr = GlobalLock(memory) as *mut u16;
        if ptr.is_null() {
            let _ = GlobalFree(memory);
            return Err("Clipboard allocation failed".into());
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), ptr, units.len());
        let _ = GlobalUnlock(memory);
        if OpenClipboard(windows::Win32::Foundation::HWND(owner.0 as _)).is_err() {
            let _ = GlobalFree(memory);
            return Err("Clipboard is busy".into());
        }
        let result =
            EmptyClipboard().and_then(|_| SetClipboardData(13, HANDLE(memory.0)).map(|_| ()));
        let _ = CloseClipboard();
        if result.is_err() {
            let _ = GlobalFree(memory);
            return Err("Could not copy clipboard item".into());
        }

        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        Err("Windows clipboard required".into())
    }
}

#[cfg(not(windows))]
pub(super) fn copy_final(_text: &str, _owner: ()) -> Result<(), String> {
    Err("Windows clipboard required".into())
}

#[cfg(test)]
mod confirmation_tests {
    #[test]
    fn confirms_full_and_partial_selection_replacement() {
        assert!(super::confirms_replacement(
            "old",
            "new",
            "new",
            Some((0, 3))
        ));
        assert!(super::confirms_replacement(
            "aoldz",
            "anewz",
            "new",
            Some((1, 4))
        ));
        assert!(super::confirms_replacement(
            "az",
            "anewz",
            "new",
            Some((1, 1))
        ));
        assert!(!super::confirms_replacement(
            "old",
            "ne",
            "new",
            Some((0, 3))
        ));
        assert!(!super::confirms_replacement("old", "new", "new", None));
        assert!(super::confirms_replacement(
            "éold",
            "é新",
            "新",
            Some((2, 5))
        ));
        assert!(!super::confirms_replacement(
            "éold",
            "é新",
            "新",
            Some((1, 5))
        ));
    }
    #[test]
    fn requires_observed_text_change() {
        assert!(super::confirms_insertion("ab", "ahellob", "hello"));
        assert!(!super::confirms_insertion("ab", "ab", "hello"));
        assert!(!super::confirms_insertion("ab", "ahelb", "hello"));
        assert!(super::confirms_insertion("?", "???", "??"));
    }
}
