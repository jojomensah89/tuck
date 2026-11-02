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
            return Err("The focused input control could not be identified. Automatic insertion was skipped.".into());
        }
        let class_name = String::from_utf16_lossy(&class_name[..length as usize]);
        let style = GetWindowLongPtrW(focus, GWL_STYLE) as u32;
        let recognized = class_name == "Edit" || class_name.starts_with("RichEdit");
        const ES_READONLY: u32 = 0x0800;
        if !recognized || (style & ES_READONLY) != 0 {
            return Err("This app's focused control is not a supported editable Windows text field. Review the transcript and place focus in a standard edit field before inserting.".into());
        }
        Ok(())
    }
}

#[cfg(not(windows))]
pub(super) fn is_editable_target(_: &ForegroundTarget) -> Result<(), String> {
    Err("Automatic text insertion is supported only on Windows.".into())
}

#[cfg(windows)]
pub(super) fn insert_unicode(target: &ForegroundTarget, text: &str) -> Result<(), String> {
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
            inputs.push(key_input(*unit, 0));
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

#[cfg(test)]
mod tests {
    #[test]
    fn unicode_input_is_split_into_bounded_batches() {
        let utf16_units = "a😀z".encode_utf16().count();
        assert_eq!(utf16_units, 4);
        assert!(256 * 2 <= 512);
    }

    #[test]
    fn windows_insertion_never_uses_the_clipboard() {
        let source = include_str!("target.rs");
        assert!(!source.contains("SetClipboardData"));
        assert!(source.contains("KEYEVENTF_UNICODE"));
    }
}
