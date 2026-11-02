//! Opt-in text-only clipboard history. It never stores clipboard images/files.
//! The background watcher reads the OS clipboard only while enabled.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

const EVENT_CHANGED: &str = "clipboard_history_changed";
const EVENT_ERROR: &str = "clipboard_history_error";
const MAX_ITEMS: usize = 40;
const MAX_ITEM_BYTES: usize = 32 * 1024;
const MAX_TOTAL_BYTES: usize = MAX_ITEMS * MAX_ITEM_BYTES;
const MAX_STORE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardHistoryItem {
    pub id: String,
    pub text: String,
    pub created_at_ms: u64,
    pub pinned: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardHistorySnapshot {
    pub enabled: bool,
    pub persist_on_exit: bool,
    pub items: Vec<ClipboardHistoryItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedSettings {
    schema_version: u32,
    enabled: bool,
    persist_on_exit: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedItems {
    schema_version: u32,
    items: Vec<ClipboardHistoryItem>,
}

struct StateInner {
    enabled: bool,
    persist_on_exit: bool,
    items: VecDeque<ClipboardHistoryItem>,
}

pub struct ClipboardHistoryService {
    app: RwLock<Option<AppHandle>>,
    inner: Mutex<StateInner>,
    stop: Arc<AtomicBool>,
    last_sequence: AtomicU32,
    operation: Mutex<()>,
}

impl Default for ClipboardHistoryService {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardHistoryService {
    pub fn new() -> Self {
        Self {
            app: RwLock::new(None),
            inner: Mutex::new(StateInner {
                enabled: false,
                persist_on_exit: false,
                items: VecDeque::new(),
            }),
            stop: Arc::new(AtomicBool::new(false)),
            last_sequence: AtomicU32::new(0),
            operation: Mutex::new(()),
        }
    }

    fn app(&self) -> Result<AppHandle, String> {
        self.app
            .read()
            .map_err(|_| "Clipboard history service unavailable".to_owned())?
            .clone()
            .ok_or_else(|| "Clipboard history has not been initialized".into())
    }

    fn snapshot(&self) -> ClipboardHistorySnapshot {
        self.inner
            .lock()
            .map(|s| ClipboardHistorySnapshot {
                enabled: s.enabled,
                persist_on_exit: s.persist_on_exit,
                items: s.items.iter().cloned().collect(),
            })
            .unwrap_or(ClipboardHistorySnapshot {
                enabled: false,
                persist_on_exit: false,
                items: vec![],
            })
    }

    fn emit_changed(&self) {
        if let Ok(app) = self.app() {
            let _ = app.emit(EVENT_CHANGED, self.snapshot());
        }
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "Clipboard settings unavailable")?;
        let app = self.app()?;
        if enabled {
            self.last_sequence
                .store(current_clipboard_sequence(), Ordering::Release);
        }
        let persist = {
            let state = self
                .inner
                .lock()
                .map_err(|_| "Clipboard history service unavailable")?;
            state.persist_on_exit
        };
        self.save_settings(&app, enabled, persist)?;
        self.inner
            .lock()
            .map_err(|_| "Clipboard settings unavailable")?
            .enabled = enabled;
        self.emit_changed();
        Ok(())
    }

    fn configure(&self, enabled: bool, persist: bool) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "Clipboard settings unavailable")?;
        let app = self.app()?;
        let previous = self.snapshot();
        if persist != previous.persist_on_exit {
            if persist {
                self.save_items(&app, &previous.items)?;
            } else {
                self.remove_items_file(&app)?;
            }
        }
        // Both preference flags commit in one atomic settings-file replacement.
        if let Err(error) = self.save_settings(&app, enabled, persist) {
            if persist != previous.persist_on_exit {
                let restored = if previous.persist_on_exit {
                    self.save_items(&app, &previous.items)
                } else {
                    self.remove_items_file(&app)
                };
                if restored.is_err() {
                    return Err("Settings were not saved and the history file could not be restored. Current preferences are unchanged.".into());
                }
            }
            return Err(error);
        }
        {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "Clipboard settings unavailable")?;
            state.enabled = enabled;
            state.persist_on_exit = persist;
        }
        if enabled {
            self.last_sequence
                .store(current_clipboard_sequence(), Ordering::Release);
        }
        self.emit_changed();
        Ok(())
    }

    fn set_persistence(&self, persist: bool) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "Clipboard settings unavailable")?;
        let app = self.app()?;
        let (enabled, items) = {
            let state = self
                .inner
                .lock()
                .map_err(|_| "Clipboard history service unavailable")?;
            (
                state.enabled,
                state.items.iter().cloned().collect::<Vec<_>>(),
            )
        };
        if persist {
            self.save_items(&app, &items)?;
        } else {
            self.remove_items_file(&app)?;
        }
        if let Err(error) = self.save_settings(&app, enabled, persist) {
            let was_persisted = self
                .inner
                .lock()
                .map_err(|_| "Clipboard settings unavailable")?
                .persist_on_exit;
            if persist && !was_persisted {
                self.remove_items_file(&app)?;
            }
            return Err(error);
        }
        self.inner
            .lock()
            .map_err(|_| "Clipboard settings unavailable")?
            .persist_on_exit = persist;
        self.emit_changed();
        Ok(())
    }

    fn push_text(&self, text: String) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "Clipboard history unavailable")?;
        if text.is_empty() || text.len() > MAX_ITEM_BYTES || text.trim().is_empty() {
            return Ok(());
        }
        let app = self.app()?;
        let state = self
            .inner
            .lock()
            .map_err(|_| "Clipboard history service unavailable")?;
        if !state.enabled {
            return Ok(());
        }
        if state.items.front().is_some_and(|item| item.text == text) {
            return Ok(());
        }
        let mut next = state.items.clone();
        next.push_front(ClipboardHistoryItem {
            id: Uuid::new_v4().to_string(),
            text,
            created_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            pinned: false,
        });
        evict_unpinned(&mut next);
        let snapshot = ClipboardHistorySnapshot {
            enabled: state.enabled,
            persist_on_exit: state.persist_on_exit,
            items: next.iter().cloned().collect(),
        };
        drop(state);
        if snapshot.persist_on_exit {
            self.save_items(&app, &snapshot.items)?;
        }
        self.inner
            .lock()
            .map_err(|_| "Clipboard history unavailable")?
            .items = next;
        self.emit_changed();
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "Clipboard history unavailable")?;
        let app = self.app()?;
        let items = {
            let state = self
                .inner
                .lock()
                .map_err(|_| "Clipboard history service unavailable")?;
            let mut next = state.items.clone();
            let before = next.len();
            next.retain(|item| item.id != id);
            if before == next.len() {
                return Err("Clipboard history item was not found".into());
            }
            next.iter().cloned().collect::<Vec<_>>()
        };
        self.persist_current(&app, &items)?;
        self.inner
            .lock()
            .map_err(|_| "Clipboard history unavailable")?
            .items = items.into_iter().collect();
        self.emit_changed();
        Ok(())
    }

    fn pin(&self, id: &str, pinned: bool) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "Clipboard history unavailable")?;
        let app = self.app()?;
        let items = {
            let state = self
                .inner
                .lock()
                .map_err(|_| "Clipboard history service unavailable")?;
            let mut next = state.items.clone();
            let item = next
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or("Clipboard history item was not found")?;
            item.pinned = pinned;
            evict_unpinned(&mut next);
            next.iter().cloned().collect::<Vec<_>>()
        };
        self.persist_current(&app, &items)?;
        self.inner
            .lock()
            .map_err(|_| "Clipboard history unavailable")?
            .items = items.into_iter().collect();
        self.emit_changed();
        Ok(())
    }

    fn clear(&self) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "Clipboard history unavailable")?;
        let app = self.app()?;
        self.remove_items_file(&app)?;
        if let Ok(mut state) = self.inner.lock() {
            state.items.clear();
        }
        self.emit_changed();
        Ok(())
    }

    fn persist_current(
        &self,
        app: &AppHandle,
        items: &[ClipboardHistoryItem],
    ) -> Result<(), String> {
        let persist = self
            .inner
            .lock()
            .map(|s| s.persist_on_exit)
            .unwrap_or(false);
        if persist {
            self.save_items(app, items)
        } else {
            Ok(())
        }
    }

    fn directory(&self, app: &AppHandle) -> Result<PathBuf, String> {
        Ok(app
            .path()
            .app_data_dir()
            .map_err(|e| format!("Could not resolve clipboard-history directory: {e}"))?
            .join("clipboard-history"))
    }

    fn save_settings(&self, app: &AppHandle, enabled: bool, persist: bool) -> Result<(), String> {
        let dir = self.directory(app)?;
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Could not create clipboard-history directory: {e}"))?;
        let stored = PersistedSettings {
            schema_version: 1,
            enabled,
            persist_on_exit: persist,
        };
        atomic_json(&dir.join("settings.json"), &stored)
    }

    fn save_items(&self, app: &AppHandle, items: &[ClipboardHistoryItem]) -> Result<(), String> {
        let dir = self.directory(app)?;
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Could not create clipboard-history directory: {e}"))?;
        let stored = PersistedItems {
            schema_version: 1,
            items: items.to_vec(),
        };
        atomic_json(&dir.join("items.json"), &stored)
    }

    fn remove_items_file(&self, app: &AppHandle) -> Result<(), String> {
        let dir = self.directory(app)?;
        match std::fs::remove_file(dir.join("items.json")) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Could not remove saved clipboard history".into()),
        }
    }
}

fn atomic_json(path: &std::path::Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|e| format!("Could not serialize clipboard history: {e}"))?;
    if bytes.len() > MAX_STORE_BYTES {
        return Err("Clipboard history exceeds the local storage limit".into());
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("Could not write clipboard history: {e}"))?;
    std::fs::rename(tmp, path).map_err(|e| format!("Could not save clipboard history: {e}"))
}

fn evict_unpinned(items: &mut VecDeque<ClipboardHistoryItem>) {
    while items.len() > MAX_ITEMS
        || items.iter().map(|item| item.text.len()).sum::<usize>() > MAX_TOTAL_BYTES
    {
        let Some(index) = items.iter().rposition(|item| !item.pinned) else {
            break;
        };
        items.remove(index);
    }
}

/// Initialize the polling worker; disabled by default, so it does not inspect
/// the OS clipboard until the user enables history.
pub fn start(app: AppHandle, service: &Arc<ClipboardHistoryService>) -> Result<(), String> {
    if let Ok(mut slot) = service.app.write() {
        *slot = Some(app.clone());
    }
    if let Ok(dir) = service.directory(&app) {
        if let Ok(bytes) = std::fs::read(dir.join("settings.json")) {
            if bytes.len() <= 4096 {
                if let Ok(settings) = serde_json::from_slice::<PersistedSettings>(&bytes) {
                    if settings.schema_version == 1 {
                        if let Ok(mut state) = service.inner.lock() {
                            state.enabled = settings.enabled;
                            state.persist_on_exit = settings.persist_on_exit;
                        }
                        if settings.persist_on_exit {
                            if let Ok(bytes) = std::fs::read(dir.join("items.json")) {
                                if bytes.len() <= MAX_STORE_BYTES {
                                    if let Ok(stored) =
                                        serde_json::from_slice::<PersistedItems>(&bytes)
                                    {
                                        if stored.schema_version == 1 && valid_items(&stored.items)
                                        {
                                            if let Ok(mut state) = service.inner.lock() {
                                                state.items = stored.items.into_iter().collect();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let worker_service = service.clone();
    let stop = service.stop.clone();
    service
        .last_sequence
        .store(current_clipboard_sequence(), Ordering::Release);
    std::thread::Builder::new()
        .name("tuck-clipboard-history".into())
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let enabled = worker_service
                    .inner
                    .lock()
                    .map(|s| s.enabled)
                    .unwrap_or(false);
                if enabled {
                    match read_clipboard_text() {
                        Ok(Some((sequence, text))) => {
                            let previous = worker_service
                                .last_sequence
                                .swap(sequence, Ordering::AcqRel);
                            if sequence != previous {
                                if let Err(error) = worker_service.push_text(text) {
                                    if let Ok(app) = worker_service.app() {
                                        let _ = app.emit(EVENT_ERROR, error);
                                    }
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            if let Ok(app) = worker_service.app() {
                                let _ = app.emit(EVENT_ERROR, error);
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(350));
            }
        })
        .map_err(|e| format!("Could not start clipboard watcher: {e}"))?;
    service.emit_changed();
    Ok(())
}

/// Stop clipboard polling during the app exit path.
pub fn stop(service: &Arc<ClipboardHistoryService>) {
    service.stop.store(true, Ordering::Release);
}

fn valid_items(items: &[ClipboardHistoryItem]) -> bool {
    items.len() <= MAX_ITEMS
        && items.iter().all(|item| {
            !item.id.is_empty()
                && item.id.len() <= 64
                && item.text.len() <= MAX_ITEM_BYTES
                && !item.text.contains('\0')
        })
        && items.iter().map(|item| item.text.len()).sum::<usize>() <= MAX_TOTAL_BYTES
}

#[cfg(windows)]
fn read_clipboard_text() -> Result<Option<(u32, String)>, String> {
    use windows::Win32::{
        Foundation::HGLOBAL,
        System::{
            DataExchange::{
                CloseClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardOwner,
                GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Threading::GetCurrentProcessId,
        },
        UI::WindowsAndMessaging::GetWindowThreadProcessId,
    };
    unsafe {
        let before = GetClipboardSequenceNumber();
        if OpenClipboard(None).is_err() {
            return Ok(None);
        }
        let result = (|| {
            let owner = GetClipboardOwner().unwrap_or_default();
            if !owner.0.is_null() {
                let mut owner_pid = 0;
                let _ = GetWindowThreadProcessId(owner, Some(&mut owner_pid));
                if owner_pid == GetCurrentProcessId() {
                    return Ok(None);
                }
            }
            if has_private_marker("ExcludeClipboardContentFromMonitorProcessing")?
                || has_disabled_history_marker("CanIncludeInClipboardHistory")?
                || has_private_marker("PasswordProtected")?
                || has_private_marker("TuckDictationTransient")?
            {
                return Ok(None);
            }
            let handle = match GetClipboardData(13) {
                Ok(handle) => HGLOBAL(handle.0),
                Err(_) => return Ok(None),
            };
            let bytes = GlobalSize(handle);
            if !(2..=MAX_ITEM_BYTES + 2).contains(&bytes) || !bytes.is_multiple_of(2) {
                return Ok(None);
            }
            let pointer = GlobalLock(handle) as *const u16;
            if pointer.is_null() {
                return Ok(None);
            }
            let units = std::slice::from_raw_parts(pointer, bytes / 2);
            let length = units
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(units.len());
            let text = String::from_utf16_lossy(&units[..length]);
            let _ = GlobalUnlock(handle);
            let after = GetClipboardSequenceNumber();
            if before != after {
                return Ok(None);
            }
            Ok(Some((after, text)))
        })();
        let close = CloseClipboard();
        if close.is_err() {
            return Err("Could not close the Windows clipboard after reading it".into());
        }
        return result;
    }

    unsafe fn has_private_marker(name: &str) -> Result<bool, String> {
        let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let id = RegisterClipboardFormatW(windows::core::PCWSTR(name.as_ptr()));
        if id == 0 {
            return Ok(false);
        }
        let mut format = 0;
        loop {
            format = EnumClipboardFormats(format);
            if format == 0 {
                break;
            }
            if format == id {
                return Ok(true);
            }
        }
        Ok(false)
    }

    unsafe fn has_disabled_history_marker(name: &str) -> Result<bool, String> {
        let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let id = RegisterClipboardFormatW(windows::core::PCWSTR(name.as_ptr()));
        if id == 0 {
            return Ok(false);
        }
        let mut format = 0;
        loop {
            format = EnumClipboardFormats(format);
            if format == 0 {
                break;
            }
            if format == id {
                let Ok(handle) = GetClipboardData(id) else {
                    return Ok(true);
                };
                let memory = HGLOBAL(handle.0);
                if GlobalSize(memory) < 4 {
                    return Ok(true);
                }
                let pointer = GlobalLock(memory) as *const u32;
                if pointer.is_null() {
                    return Ok(true);
                }
                let value = *pointer;
                let _ = GlobalUnlock(memory);
                return Ok(value == 0);
            }
        }
        Ok(false)
    }
}

#[tauri::command]
pub fn clipboard_copy(
    window: tauri::Window,
    service: State<'_, Arc<ClipboardHistoryService>>,
    id: String,
) -> Result<(), String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Access denied".into());
    }
    let text = service
        .inner
        .lock()
        .map_err(|_| "Clipboard history unavailable")?
        .items
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.text.clone())
        .ok_or("Clipboard item not found")?;
    #[cfg(windows)]
    unsafe {
        use windows::Win32::{
            Foundation::{GlobalFree, HANDLE},
            System::{
                DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
                Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            },
        };
        let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let memory = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2)
            .map_err(|_| "Clipboard allocation failed")?;
        let ptr = GlobalLock(memory) as *mut u16;
        if ptr.is_null() {
            let _ = GlobalFree(memory);
            return Err("Clipboard allocation failed".into());
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), ptr, units.len());
        let _ = GlobalUnlock(memory);
        let owner = window.hwnd().map_err(|_| "Clipboard window unavailable")?;
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
        service
            .last_sequence
            .store(current_clipboard_sequence(), Ordering::Release);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        Err("Windows clipboard required".into())
    }
}

#[cfg(windows)]
fn current_clipboard_sequence() -> u32 {
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    unsafe { GetClipboardSequenceNumber() }
}

#[cfg(not(windows))]
fn read_clipboard_text() -> Result<Option<(u32, String)>, String> {
    Ok(None)
}

#[cfg(not(windows))]
fn current_clipboard_sequence() -> u32 {
    0
}

#[tauri::command]
pub fn clipboard_get_state(
    service: State<'_, Arc<ClipboardHistoryService>>,
) -> ClipboardHistorySnapshot {
    service.snapshot()
}

#[tauri::command]
pub fn clipboard_configure(
    window: tauri::Window,
    service: State<'_, Arc<ClipboardHistoryService>>,
    enabled: bool,
    persist_on_exit: bool,
) -> Result<ClipboardHistorySnapshot, String> {
    if !matches!(window.label(), "main" | "settings") {
        return Err("Access denied".into());
    }
    service.configure(enabled, persist_on_exit)?;
    Ok(service.snapshot())
}

#[tauri::command]
pub fn clipboard_set_enabled(
    service: State<'_, Arc<ClipboardHistoryService>>,
    enabled: bool,
) -> Result<ClipboardHistorySnapshot, String> {
    service.set_enabled(enabled)?;
    Ok(service.snapshot())
}

#[tauri::command]
pub fn clipboard_set_persistence(
    service: State<'_, Arc<ClipboardHistoryService>>,
    persist_on_exit: bool,
) -> Result<ClipboardHistorySnapshot, String> {
    service.set_persistence(persist_on_exit)?;
    Ok(service.snapshot())
}

#[tauri::command]
pub fn clipboard_delete(
    service: State<'_, Arc<ClipboardHistoryService>>,
    id: String,
) -> Result<ClipboardHistorySnapshot, String> {
    if id.len() > 64 {
        return Err("Clipboard history item ID is too long".into());
    }
    service.delete(&id)?;
    Ok(service.snapshot())
}

#[tauri::command]
pub fn clipboard_clear(
    service: State<'_, Arc<ClipboardHistoryService>>,
) -> Result<ClipboardHistorySnapshot, String> {
    service.clear()?;
    Ok(service.snapshot())
}

#[tauri::command]
pub fn clipboard_pin(
    service: State<'_, Arc<ClipboardHistoryService>>,
    id: String,
    pinned: bool,
) -> Result<ClipboardHistorySnapshot, String> {
    if id.len() > 64 {
        return Err("Clipboard history item ID is too long".into());
    }
    service.pin(&id, pinned)?;
    Ok(service.snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn failed_atomic_settings_replace_preserves_both_preferences() {
        use std::io::Read;
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("tuck-clipboard-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("settings.json");
        for enabled in [false, true, false] {
            atomic_json(
                &path,
                &PersistedSettings {
                    schema_version: 1,
                    enabled,
                    persist_on_exit: enabled,
                },
            )
            .unwrap();
        }
        let mut locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(atomic_json(
            &path,
            &PersistedSettings {
                schema_version: 1,
                enabled: true,
                persist_on_exit: true
            }
        )
        .is_err());
        let mut bytes = Vec::new();
        locked.read_to_end(&mut bytes).unwrap();
        let saved: PersistedSettings = serde_json::from_slice(&bytes).unwrap();
        assert!(!saved.enabled && !saved.persist_on_exit);
        drop(locked);
        std::fs::remove_file(&path).unwrap();
        let _ = std::fs::remove_file(path.with_extension("json.tmp"));
        std::fs::remove_dir(dir).unwrap();
    }

    fn item(pinned: bool, text: &str) -> ClipboardHistoryItem {
        ClipboardHistoryItem {
            id: Uuid::new_v4().to_string(),
            text: text.into(),
            created_at_ms: 0,
            pinned,
        }
    }

    #[test]
    fn bounded_history_evicts_oldest_unpinned_first() {
        let mut items = VecDeque::new();
        items.push_front(item(true, "keep"));
        for index in 0..MAX_ITEMS {
            items.push_front(item(false, &index.to_string()));
        }
        evict_unpinned(&mut items);
        assert_eq!(items.len(), MAX_ITEMS);
        assert!(items
            .iter()
            .any(|entry| entry.pinned && entry.text == "keep"));
    }

    #[test]
    fn persistence_validation_rejects_oversize_and_nul_input() {
        assert!(!valid_items(&[item(
            false,
            &"x".repeat(MAX_ITEM_BYTES + 1)
        )]));
        assert!(!valid_items(&[item(false, "unsafe\0text")]));
    }

    #[test]
    fn default_state_is_disabled_and_ephemeral() {
        let service = ClipboardHistoryService::new();
        let snapshot = service.snapshot();
        assert!(!snapshot.enabled);
        assert!(!snapshot.persist_on_exit);
        assert!(snapshot.items.is_empty());
    }
}
