//! Windows WebView2 can replace its renderer HWNDs after Wry's one-time registration.
//! Refresh only on activity and native child creation; no timer or webview idle work.
use std::{cell::Cell, ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
#[cfg(debug_assertions)]
use tauri::Manager;
use tauri::{Emitter, EventTarget, WebviewWindow};
use windows::{
    core::{implement, Result},
    Win32::{
        Foundation::{BOOL, HWND, LPARAM, POINTL},
        System::{
            Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL},
            Ole::{
                IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
                CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE,
            },
        },
        UI::{
            Shell::{DragQueryFileW, HDROP},
            WindowsAndMessaging::EnumChildWindows,
        },
    },
};

#[implement(IDropTarget)]
struct Target {
    window: WebviewWindow,
    valid: Cell<bool>,
}
fn paths(data: Option<&IDataObject>) -> Option<Vec<PathBuf>> {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        ..Default::default()
    };
    unsafe {
        let mut medium = data?.GetData(&format).ok()?;
        if medium.tymed != TYMED_HGLOBAL.0 as u32 {
            ReleaseStgMedium(&mut medium);
            return None;
        }
        let drop = HDROP(medium.u.hGlobal.0);
        let mut paths = Vec::new();
        for i in 0..DragQueryFileW(drop, u32::MAX, None) {
            let len = DragQueryFileW(drop, i, None) as usize;
            let mut name = vec![0; len + 1];
            DragQueryFileW(drop, i, Some(&mut name));
            paths.push(OsString::from_wide(&name[..len]).into());
        }
        ReleaseStgMedium(&mut medium);
        (!paths.is_empty()).then_some(paths)
    }
}
impl Target {
    fn emit(&self, event: &str, point: &POINTL, paths: Option<Vec<PathBuf>>) {
        let position = self.window.inner_position().unwrap_or_default();
        let _ = self.window.emit_to(EventTarget::labeled(self.window.label()), event, serde_json::json!({"paths":paths,"position":{"x":point.x-position.x,"y":point.y-position.y}}));
    }
}
#[allow(non_snake_case)]
impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Option<&IDataObject>,
        _keys: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let files = paths(data);
        self.valid.set(files.is_some());
        unsafe {
            *effect = if self.valid.get() {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        if self.valid.get() {
            self.emit("tauri://drag-enter", point, files);
        }
        Ok(())
    }
    fn DragOver(
        &self,
        _keys: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        unsafe {
            *effect = if self.valid.get() {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        if self.valid.get() {
            self.emit("tauri://drag-over", point, None);
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        if self.valid.replace(false) {
            let _ = self.window.emit_to(
                EventTarget::labeled(self.window.label()),
                "tauri://drag-leave",
                (),
            );
        }
        Ok(())
    }
    fn Drop(
        &self,
        data: Option<&IDataObject>,
        _keys: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let files = paths(data);
        unsafe {
            *effect = if files.is_some() {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        if files.is_some() {
            self.emit("tauri://drag-drop", point, files);
        }
        self.valid.set(false);
        Ok(())
    }
}

// Allows the identical isolated executable/probe to measure Wry's original path.
// This switch cannot affect release builds or the normal app identifier.
fn baseline(window: &WebviewWindow) -> bool {
    #[cfg(debug_assertions)]
    {
        window.app_handle().config().identifier == "com.jojo.tuck.drop-probe"
            && std::env::var_os("TUCK_DROP_BASELINE").is_some()
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = window;
        false
    }
}
pub(crate) fn refresh(window: &WebviewWindow) {
    if baseline(window) {
        return;
    }
    let window = window.clone();
    let handle = window.clone();
    let _ = handle.run_on_main_thread(move || {
        let Ok(root) = window.hwnd() else {
            return;
        };
        unsafe extern "system" fn child(hwnd: HWND, param: LPARAM) -> BOOL {
            let window = unsafe { &*(param.0 as *const WebviewWindow) };
            // Keep a target's DragEnter state alive across the opening activity update.
            // Replacing a registered target mid-drag can deliver Over without Enter.
            if TARGETS.with(|targets| targets.borrow().contains_key(&(hwnd.0 as isize))) {
                return BOOL(1);
            }
            let target: IDropTarget = Target {
                window: window.clone(),
                valid: Cell::new(false),
            }
            .into();
            unsafe {
                let _ = RevokeDragDrop(hwnd);
                match RegisterDragDrop(hwnd, &target) {
                    Ok(()) => TARGETS.with(|targets| {
                        targets.borrow_mut().insert(hwnd.0 as isize, target);
                    }),
                    Err(error) => eprintln!("File drop registration: {error}"),
                }
            }
            BOOL(1)
        }
        unsafe {
            let _ = EnumChildWindows(
                HWND(root.0),
                Some(child),
                LPARAM(&window as *const _ as isize),
            );
        }
    });
}

thread_local! {
    static TARGETS: std::cell::RefCell<std::collections::HashMap<isize, IDropTarget>> = std::cell::RefCell::new(std::collections::HashMap::new());
    static WATCH: std::cell::RefCell<Option<(WebviewWindow, windows::Win32::UI::Accessibility::HWINEVENTHOOK)>> = const { std::cell::RefCell::new(None) };
}
pub(crate) fn start(window: &WebviewWindow) {
    if baseline(window) {
        return;
    }
    use windows::Win32::UI::{
        Accessibility::SetWinEventHook,
        WindowsAndMessaging::{EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, WINEVENT_OUTOFCONTEXT},
    };
    unsafe extern "system" fn created(
        _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        object: i32,
        child: i32,
        _thread: u32,
        _time: u32,
    ) {
        if object != 0 || child != 0 {
            return;
        }
        if event == windows::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_DESTROY {
            TARGETS.with(|targets| {
                targets.borrow_mut().remove(&(hwnd.0 as isize));
            });
            return;
        }
        let window = WATCH.with(|watch| watch.borrow().as_ref().map(|(window, _)| window.clone()));
        if let Some(window) = window {
            if window.hwnd().is_ok_and(|root| unsafe {
                windows::Win32::UI::WindowsAndMessaging::IsChild(HWND(root.0), hwnd).as_bool()
            }) {
                refresh(&window);
            }
        }
    }
    let hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_CREATE,
            EVENT_OBJECT_DESTROY,
            None,
            Some(created),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    WATCH.with(|watch| *watch.borrow_mut() = Some((window.clone(), hook)));
    refresh(window);
}
pub(crate) fn stop() {
    TARGETS.with(|targets| targets.borrow_mut().clear());
    if let Some((_, hook)) = WATCH.with(|watch| watch.borrow_mut().take()) {
        unsafe {
            let _ = windows::Win32::UI::Accessibility::UnhookWinEvent(hook);
        }
    }
}
