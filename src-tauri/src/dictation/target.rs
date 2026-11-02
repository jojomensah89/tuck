//! Conservative Windows target capture and Unicode insertion.
//! Clipboard is never touched: text is sent with Unicode keyboard input, so
//! concurrent clipboard changes are preserved without a restore race.

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
            CUIAutomation, IUIAutomation, IUIAutomationValuePattern, UIA_EditControlTypeId,
            UIA_ValuePatternId,
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
            if !editable || focused.CurrentControlType().ok() != Some(UIA_EditControlTypeId) {
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
pub(super) fn insert_unicode(target: &ForegroundTarget, text: &str) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        .any(|key| unsafe { GetAsyncKeyState(key.0 as i32) } < 0)
    {
        if std::time::Instant::now() > deadline {
            return Err("Release shortcut keys before inserting.".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    verify_foreground_target(target)?;
    is_editable_target(target)?;
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() > 32 * 1024 {
        return Err("Transcript is too long to insert safely".into());
    }
    for chunk in units.chunks(256) {
        verify_foreground_target(target)?;
        let mut inputs = Vec::with_capacity(chunk.len() * 2);
        for unit in chunk {
            inputs.push(key_input(*unit, Default::default()));
            inputs.push(key_input(*unit, KEYEVENTF_KEYUP));
        }
        let inserted = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if inserted != inputs.len() as u32 {
            return Err("Windows accepted only part of the text insertion. Check the target app's integrity level and input support.".into());
        }
    }
    Ok(())
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
pub(super) fn insert_unicode(_: &ForegroundTarget, _: &str) -> Result<(), String> {
    Err("Automatic text insertion is supported only on Windows.".into())
}
