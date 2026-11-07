//! Opt-in, bounded text, image and file-path clipboard history.
//! The background watcher reads the OS clipboard only while enabled.

#[path = "clipboard_media.rs"]
mod media;

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

static CLIPBOARD_WAKE: Mutex<Option<std::sync::mpsc::Sender<()>>> = Mutex::new(None);
pub(crate) fn clipboard_updated() {
    if let Some(tx) = CLIPBOARD_WAKE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
    {
        let _ = tx.send(());
    }
}
pub(crate) fn listener_enabled(app: &AppHandle) -> bool {
    app.state::<Arc<ClipboardHistoryService>>()
        .inner
        .lock()
        .map(|state| state.enabled)
        .unwrap_or(false)
}

#[derive(Debug, PartialEq)]
enum ClipboardRead {
    Busy,
    Ignored,
    Text(u32, String),
    Media(u32, media::Capture),
}

// Ten reads per sequence: at most 5.24 seconds of scheduled backoff, then no timer.
const CAPTURE_ATTEMPTS: u8 = 10;
#[derive(Default)]
struct CaptureRetry {
    delay: Option<std::time::Duration>,
    sequence: Option<(u32, u64)>,
    remaining: u8,
}
impl CaptureRetry {
    fn notified(&mut self, sequence: u32, generation: u64) -> bool {
        // Duplicate notifications cannot renew an exhausted sequence's budget.
        if self.sequence == Some((sequence, generation)) {
            return false;
        }
        self.sequence = Some((sequence, generation));
        self.remaining = CAPTURE_ATTEMPTS;
        self.delay = Some(std::time::Duration::from_millis(20));
        true
    }
    fn completed(&mut self) {
        self.delay = None;
        self.remaining = 0;
    }
    fn wait(
        &self,
        rx: &std::sync::mpsc::Receiver<()>,
    ) -> Result<(), std::sync::mpsc::RecvTimeoutError> {
        match self.delay {
            Some(delay) => rx.recv_timeout(delay),
            None => rx
                .recv()
                .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected),
        }
    }
    fn observed(&mut self, result: &Result<ClipboardRead, String>) {
        self.remaining = self.remaining.saturating_sub(1);
        if matches!(result, Ok(ClipboardRead::Busy)) && self.remaining > 0 {
            self.delay = Some(
                (self.delay.unwrap_or(std::time::Duration::from_millis(20)) * 2)
                    .min(std::time::Duration::from_secs(1)),
            );
        } else {
            self.completed();
        }
    }
}

const EVENT_CHANGED: &str = "clipboard_history_changed";
const EVENT_ERROR: &str = "clipboard_history_error";
const MAX_ITEMS: usize = 40;
const MAX_ITEM_BYTES: usize = 32 * 1024;
const MAX_TOTAL_BYTES: usize = MAX_ITEMS * MAX_ITEM_BYTES;
const MAX_STORE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardHistoryItem {
    pub id: String,
    pub text: String,
    pub created_at_ms: u64,
    pub pinned: bool,
    #[serde(default = "text_kind")]
    pub kind: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(skip)]
    pub image: Option<Arc<Vec<u8>>>,
    #[serde(default)]
    pub missing: bool,
}

fn text_kind() -> String {
    "text".into()
}
impl ClipboardHistoryItem {
    fn new(text: String) -> Self {
        let kind = if !text.chars().any(char::is_whitespace)
            && text.split_once("://").is_some_and(|(scheme, _)| {
                scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
            })
            && reqwest::Url::parse(&text).is_ok_and(|url| url.host_str().is_some())
        {
            "link"
        } else {
            "text"
        };
        Self {
            id: Uuid::new_v4().to_string(),
            text,
            kind: kind.into(),
            paths: vec![],
            thumbnail: None,
            image: None,
            missing: false,
            created_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            pinned: false,
        }
    }
}

impl Drop for ClipboardHistoryItem {
    fn drop(&mut self) {
        self.id.zeroize();
        self.text.zeroize();
        self.paths.zeroize();
        if let Some(thumbnail) = &mut self.thumbnail {
            thumbnail.zeroize();
        }
    }
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
    generation: u64,
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
                generation: 0,
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
                items: s
                    .items
                    .iter()
                    .cloned()
                    .map(|mut item| {
                        item.missing = media::missing(&item);
                        item
                    })
                    .collect(),
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
        if !enabled {
            self.remove_items_file(&app)?;
        }
        self.save_settings(&app, enabled, persist)?;
        {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "Clipboard settings unavailable")?;
            update_enabled(&mut state, enabled);
        }
        clipboard_updated();
        crate::lifecycle::request_refresh();
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
        if !enabled {
            self.remove_items_file(&app)?;
        }
        if persist != previous.persist_on_exit {
            if persist && enabled {
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
            update_enabled(&mut state, enabled);
            state.persist_on_exit = persist;
        }
        if enabled {
            self.last_sequence
                .store(current_clipboard_sequence(), Ordering::Release);
        }
        clipboard_updated();
        crate::lifecycle::request_refresh();
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
        if persist && enabled {
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
        clipboard_updated();
        crate::lifecycle::request_refresh();
        self.emit_changed();
        Ok(())
    }

    fn push_item(&self, item: ClipboardHistoryItem, generation: u64) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "Clipboard history unavailable")?;
        if !valid_items(std::slice::from_ref(&item)) {
            return Ok(());
        }
        let app = self.app()?;
        let state = self
            .inner
            .lock()
            .map_err(|_| "Clipboard history service unavailable")?;
        if !accept_clipboard_generation(state.enabled, state.generation, generation) {
            return Ok(());
        }
        if state.items.front().is_some_and(|previous| {
            previous.kind == item.kind
                && previous.text == item.text
                && previous.paths == item.paths
                && previous.image == item.image
        }) {
            return Ok(());
        }
        let mut next = state.items.clone();
        next.push_front(item);
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
        clipboard_updated();
        crate::lifecycle::request_refresh();
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
        clipboard_updated();
        crate::lifecycle::request_refresh();
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
        clipboard_updated();
        crate::lifecycle::request_refresh();
        self.emit_changed();
        Ok(())
    }

    fn clear(&self) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| "Clipboard history unavailable")?;
        let app = self.app()?;
        let items = self
            .inner
            .lock()
            .map_err(|_| "Clipboard history unavailable")?
            .items
            .iter()
            .filter(|item| item.pinned)
            .cloned()
            .collect::<Vec<_>>();
        if items.is_empty() {
            self.remove_items_file(&app)?;
        } else {
            self.persist_current(&app, &items)?;
        }
        self.inner
            .lock()
            .map_err(|_| "Clipboard history unavailable")?
            .items = items.into_iter().collect();
        clipboard_updated();
        crate::lifecycle::request_refresh();
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
        save_history(&dir, items)
    }

    fn remove_items_file(&self, app: &AppHandle) -> Result<(), String> {
        remove_history(&self.directory(app)?)
    }
}

// Sidecars are immutable, UUID-named, and only created with persistence enabled.
fn cleanup_images(dir: &std::path::Path, items: &[ClipboardHistoryItem]) -> Result<(), String> {
    let media_dir = dir.join("images");
    let entries = match std::fs::read_dir(&media_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("Could not clean saved clipboard images".into()),
    };
    for entry in entries {
        let entry = entry.map_err(|_| "Could not clean saved clipboard images")?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let managed = name
            .strip_suffix(".dib")
            .or_else(|| name.strip_suffix(".tmp"));
        if let Some(id) = managed.filter(|id| Uuid::parse_str(id).is_ok()) {
            if name.ends_with(".tmp")
                || !items
                    .iter()
                    .any(|item| item.id == id && item.kind == "image")
            {
                remove_file_if_exists(&entry.path())
                    .map_err(|_| "Could not remove saved clipboard image")?;
            }
        }
    }
    Ok(())
}
fn cleanup_history_temps(dir: &std::path::Path) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("Could not clean clipboard temporary files".into()),
    };
    for entry in entries {
        let entry = entry.map_err(|_| "Could not clean clipboard temporary files")?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name
            .strip_prefix("items.json.")
            .and_then(|name| name.strip_suffix(".tmp"))
            .is_some_and(|id| Uuid::parse_str(id).is_ok())
        {
            remove_file_if_exists(&entry.path())
                .map_err(|_| "Could not remove clipboard temporary file")?;
        }
    }
    Ok(())
}
fn remove_history(dir: &std::path::Path) -> Result<(), String> {
    // Remove the index first: interrupted cleanup cannot resurrect removed content.
    remove_file_if_exists(&dir.join("items.json"))
        .map_err(|_| "Could not remove saved clipboard history")?;
    cleanup_history_temps(dir)?;
    cleanup_images(dir, &[])
}
fn save_history(dir: &std::path::Path, items: &[ClipboardHistoryItem]) -> Result<(), String> {
    if !valid_items(items) || items.iter().map(media::bytes).sum::<usize>() > media::MAX_IMAGES {
        return Err("Clipboard storage limit exceeded".into());
    }
    std::fs::create_dir_all(dir).map_err(|_| "Could not create clipboard directory")?;
    cleanup_history_temps(dir)?;
    // Preserve the old manifest and its immutable blobs until the new index commits.
    // The image budget reserves 49 MiB for one new bitmap, both metadata versions,
    // and settings, including temporary files, inside the 256 MiB disk cap.
    let previous = match read_bounded(&dir.join("items.json"), MAX_STORE_BYTES) {
        Ok(data) => {
            let stored: PersistedItems = serde_json::from_slice(&data)
                .map_err(|_| "Could not read saved clipboard index")?;
            if !valid_persisted_items(&stored) {
                return Err("Invalid saved clipboard index".into());
            }
            stored.items
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return Err("Could not read saved clipboard index".into()),
    };
    cleanup_images(dir, &previous)?;
    let images = dir.join("images");
    let existing = if images.exists() {
        std::fs::read_dir(&images)
            .map_err(|_| "Could not inspect clipboard storage")?
            .try_fold(0u64, |sum, entry| {
                let entry = entry?;
                Ok::<_, std::io::Error>(sum.saturating_add(entry.metadata()?.len()))
            })
            .map_err(|_| "Could not inspect clipboard storage")?
    } else {
        0
    };
    let additional: usize = items
        .iter()
        .filter(|item| !images.join(format!("{}.dib", item.id)).exists())
        .map(|item| item.image.as_ref().map_or(0, |image| image.len()))
        .sum();
    if existing + additional as u64 + 2 * MAX_STORE_BYTES as u64 + 4096 > 256 * 1024 * 1024 {
        return Err("Clipboard storage limit exceeded".into());
    }
    let result = (|| {
        for item in items {
            if let Some(image) = &item.image {
                std::fs::create_dir_all(&images).map_err(|_| "Could not save clipboard image")?;
                let path = images.join(format!("{}.dib", item.id));
                if !path.exists() {
                    let temp = images.join(format!("{}.tmp", item.id));
                    let _cleanup = TemporaryHistoryFile(temp.clone());
                    std::fs::write(&temp, image.as_slice())
                        .map_err(|_| "Could not save clipboard image")?;
                    replace_file(&temp, &path).map_err(|_| "Could not save clipboard image")?;
                }
            }
        }
        atomic_json(
            &dir.join("items.json"),
            &PersistedItems {
                schema_version: 1,
                items: items.to_vec(),
            },
        )
    })();
    if result.is_ok() {
        cleanup_images(dir, items)?;
    } else {
        cleanup_images(dir, &previous)?;
    }
    result
}

fn restore_images(
    dir: &std::path::Path,
    items: &mut Vec<ClipboardHistoryItem>,
) -> Result<(), String> {
    let mut total = 0;
    items.retain_mut(|item| {
        if item.kind != "image" {
            return true;
        }
        let result = read_bounded(
            &dir.join("images").join(format!("{}.dib", item.id)),
            media::MAX_CAPTURE,
        )
        .ok()
        .and_then(|bytes| media::prepare(media::Capture::Dib(bytes)).ok());
        if let Some(decoded) = result {
            if total + media::bytes(&decoded) > media::MAX_IMAGES {
                return false;
            }
            total += media::bytes(&decoded);
            item.image = decoded.image.clone();
            item.thumbnail = decoded.thumbnail.clone();
            true
        } else {
            false
        }
    });
    Ok(())
}

fn atomic_json(path: &std::path::Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = Zeroizing::new(
        serde_json::to_vec(value)
            .map_err(|e| format!("Could not serialize clipboard history: {e}"))?,
    );
    if bytes.len() > MAX_STORE_BYTES {
        return Err("Clipboard history exceeds the local storage limit".into());
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid clipboard history file path")?;
    let tmp = path.with_file_name(format!("{file_name}.{}.tmp", Uuid::new_v4()));
    let _cleanup = TemporaryHistoryFile(tmp.clone());
    std::fs::write(&tmp, bytes.as_slice())
        .map_err(|e| format!("Could not write clipboard history: {e}"))?;
    replace_file(&tmp, path).map_err(|e| format!("Could not save clipboard history: {e}"))
}

struct TemporaryHistoryFile(PathBuf);

impl Drop for TemporaryHistoryFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(windows)]
fn replace_file(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        },
    };
    let from: Vec<u16> = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let to: Vec<u16> = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| std::io::Error::other(error.to_string()))
}

#[cfg(not(windows))]
fn replace_file(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

fn evict_unpinned(items: &mut VecDeque<ClipboardHistoryItem>) {
    while items.len() > MAX_ITEMS
        || items
            .iter()
            .map(|item| item.text.len() + item.paths.iter().map(String::len).sum::<usize>())
            .sum::<usize>()
            > MAX_TOTAL_BYTES
        || items.iter().map(media::bytes).sum::<usize>() > media::MAX_IMAGES
    {
        let image_pressure = items.iter().map(media::bytes).sum::<usize>() > media::MAX_IMAGES;
        let Some(index) = items
            .iter()
            .rposition(|item| !item.pinned && (!image_pressure || item.kind == "image"))
        else {
            break;
        };
        items.remove(index);
    }
}

fn should_skip_clipboard_item(
    exclude_from_monitoring: bool,
    history_disabled: bool,
    password_marked: bool,
    tuck_transient: bool,
) -> bool {
    exclude_from_monitoring || history_disabled || password_marked || tuck_transient
}

fn clipboard_sequence_unchanged(before: u32, after: u32) -> bool {
    before == after
}

fn accept_clipboard_generation(enabled: bool, current: u64, captured: u64) -> bool {
    enabled && current == captured
}

fn accepts_clipboard_text(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_ITEM_BYTES && !text.trim().is_empty()
}

fn update_enabled(state: &mut StateInner, enabled: bool) {
    if !enabled {
        state.items.clear();
    }
    if state.enabled != enabled {
        state.enabled = enabled;
        state.generation = state.generation.wrapping_add(1);
    }
}

fn valid_persisted_items(stored: &PersistedItems) -> bool {
    stored.schema_version == 1 && valid_items(&stored.items)
}

fn read_bounded(path: &std::path::Path, max_bytes: usize) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > max_bytes as u64 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file exceeds the configured size limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file grew beyond the configured size limit",
        ));
    }
    Ok(bytes)
}

fn remove_file_if_exists(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Initialize the polling worker; disabled by default, so it does not inspect
/// the OS clipboard until the user enables history.
pub fn start(app: AppHandle, service: &Arc<ClipboardHistoryService>) -> Result<(), String> {
    *service
        .app
        .write()
        .map_err(|_| "Clipboard history service unavailable")? = Some(app.clone());
    let dir = service.directory(&app)?;
    cleanup_history_temps(&dir)?;
    let mut load_items = false;
    match read_bounded(&dir.join("settings.json"), 4096) {
        Ok(bytes) => {
            if let Ok(settings) = serde_json::from_slice::<PersistedSettings>(&bytes) {
                if settings.schema_version == 1 {
                    let mut state = service
                        .inner
                        .lock()
                        .map_err(|_| "Clipboard history service unavailable")?;
                    state.enabled = settings.enabled;
                    state.persist_on_exit = settings.persist_on_exit;
                    load_items = settings.enabled && settings.persist_on_exit;
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {}
        Err(error) => {
            return Err(format!(
                "Could not read clipboard-history settings: {error}"
            ));
        }
    }
    if load_items {
        let items_path = dir.join("items.json");
        match read_bounded(&items_path, MAX_STORE_BYTES) {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                match serde_json::from_slice::<PersistedItems>(bytes.as_slice()) {
                    Ok(mut stored) if valid_persisted_items(&stored) => {
                        restore_images(&dir, &mut stored.items)?;
                        cleanup_images(&dir, &stored.items)?;
                        service
                            .inner
                            .lock()
                            .map_err(|_| "Clipboard history service unavailable")?
                            .items = stored.items.into_iter().collect();
                    }
                    _ => {
                        remove_file_if_exists(&items_path).map_err(|error| {
                            format!("Could not remove invalid clipboard history: {error}")
                        })?;
                        cleanup_images(&dir, &[])?;
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                cleanup_images(&dir, &[])?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                remove_file_if_exists(&items_path).map_err(|error| {
                    format!("Could not remove oversized clipboard history: {error}")
                })?;
                cleanup_images(&dir, &[])?;
            }
            Err(error) => {
                return Err(format!(
                    "Could not read persisted clipboard history: {error}"
                ));
            }
        }
    } else {
        // A missing/invalid opt-in setting must not leave stale clipboard text on disk.
        service.remove_items_file(&app)?;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    *CLIPBOARD_WAKE.lock().unwrap_or_else(|p| p.into_inner()) = Some(tx);
    let worker_service = service.clone();
    let stop = service.stop.clone();
    service
        .last_sequence
        .store(current_clipboard_sequence(), Ordering::Release);
    std::thread::Builder::new()
        .name("tuck-clipboard-history".into())
        .spawn(move || {
            let mut retry = CaptureRetry::default();
            loop {
                let notified = match retry.wait(&rx) {
                    Ok(()) => true,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                };
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let (enabled, generation) = worker_service
                    .inner
                    .lock()
                    .map(|s| (s.enabled, s.generation))
                    .unwrap_or((false, 0));
                if !enabled {
                    retry.completed();
                    continue;
                }
                if notified && !retry.notified(current_clipboard_sequence(), generation) {
                    continue;
                }
                if retry.delay.is_none() {
                    continue;
                }
                let result = read_clipboard_text();
                retry.observed(&result);
                match result {
                    Ok(ClipboardRead::Busy) => {}
                    Ok(ClipboardRead::Text(sequence, text)) => {
                        retry.completed();
                        let mut text = Zeroizing::new(text);
                        let previous = worker_service
                            .last_sequence
                            .swap(sequence, Ordering::AcqRel);
                        if sequence != previous {
                            if let Err(error) = worker_service.push_item(
                                ClipboardHistoryItem::new(std::mem::take(&mut *text)),
                                generation,
                            ) {
                                if let Ok(app) = worker_service.app() {
                                    let _ = app.emit(EVENT_ERROR, error);
                                }
                            }
                        }
                    }
                    Ok(ClipboardRead::Media(sequence, capture)) => {
                        retry.completed();
                        let previous = worker_service
                            .last_sequence
                            .swap(sequence, Ordering::AcqRel);
                        if sequence != previous {
                            match media::prepare(capture) {
                                Ok(item) => {
                                    if let Err(error) = worker_service.push_item(item, generation) {
                                        if let Ok(app) = worker_service.app() {
                                            let _ = app.emit(EVENT_ERROR, error);
                                        }
                                    }
                                }
                                Err(_) => eprintln!(
                                    "Clipboard item skipped: unsupported data or size limit"
                                ),
                            }
                        }
                    }
                    Ok(ClipboardRead::Ignored) => retry.completed(),
                    Err(error) => {
                        retry.completed();
                        if let Ok(app) = worker_service.app() {
                            let _ = app.emit(EVENT_ERROR, error);
                        }
                    }
                }
            }
        })
        .map_err(|e| format!("Could not start clipboard watcher: {e}"))?;
    service.emit_changed();
    Ok(())
}

/// Stop clipboard polling during the app exit path.
pub fn stop(service: &Arc<ClipboardHistoryService>) {
    service.stop.store(true, Ordering::Release);
    clipboard_updated();
}

fn valid_items(items: &[ClipboardHistoryItem]) -> bool {
    items.len() <= MAX_ITEMS
        && items.iter().all(|item| {
            Uuid::parse_str(&item.id).is_ok()
                && item.id.len() <= 64
                && accepts_clipboard_text(&item.text)
                && item.text.len() <= MAX_ITEM_BYTES
                && !item.text.contains('\0')
                && matches!(
                    item.kind.as_str(),
                    "text" | "link" | "image" | "video" | "file"
                )
                && (item.paths.is_empty() || media::valid_paths(&item.paths))
                && (matches!(item.kind.as_str(), "video" | "file") == !item.paths.is_empty())
                && item.thumbnail.as_ref().is_none_or(|t| {
                    t.len() <= media::MAX_THUMBNAIL * 4 / 3 + 32
                        && t.starts_with("data:image/png;base64,")
                })
        })
        && items.iter().map(|item| item.text.len()).sum::<usize>() <= MAX_TOTAL_BYTES
}

#[cfg(windows)]
fn read_clipboard_text() -> Result<ClipboardRead, String> {
    use windows::Win32::{
        Foundation::HGLOBAL,
        System::{
            DataExchange::{
                CloseClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardOwner,
                GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
                RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::CF_UNICODETEXT,
            Threading::GetCurrentProcessId,
        },
        UI::WindowsAndMessaging::GetWindowThreadProcessId,
    };
    unsafe {
        let before = GetClipboardSequenceNumber();
        if OpenClipboard(None).is_err() {
            return Ok(ClipboardRead::Busy);
        }
        let mut result = (|| {
            let owner = GetClipboardOwner().unwrap_or_default();
            if !owner.0.is_null() {
                let mut owner_pid = 0;
                if GetWindowThreadProcessId(owner, Some(&mut owner_pid)) == 0 {
                    return Ok(ClipboardRead::Ignored);
                }
                if owner_pid == GetCurrentProcessId() {
                    return Ok(ClipboardRead::Ignored);
                }
            }
            let exclude_from_monitoring =
                has_private_marker("ExcludeClipboardContentFromMonitorProcessing")?;
            let history_disabled = has_disabled_history_marker("CanIncludeInClipboardHistory")?;
            let password_marked = has_private_marker("PasswordProtected")?;
            let tuck_transient = has_private_marker("TuckDictationTransient")?;
            if should_skip_clipboard_item(
                exclude_from_monitoring,
                history_disabled,
                password_marked,
                tuck_transient,
            ) {
                return Ok(ClipboardRead::Ignored);
            }
            match media::read_native() {
                Ok(Some(capture)) => {
                    let after = GetClipboardSequenceNumber();
                    return if clipboard_sequence_unchanged(before, after) {
                        Ok(ClipboardRead::Media(after, capture))
                    } else {
                        Ok(ClipboardRead::Busy)
                    };
                }
                Err(_) => {
                    eprintln!("Clipboard item skipped: unsupported data or size limit");
                    return Ok(ClipboardRead::Ignored);
                }
                Ok(None) => {}
            }
            if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
                return Ok(ClipboardRead::Ignored);
            }
            let handle = match GetClipboardData(CF_UNICODETEXT.0 as u32) {
                Ok(handle) => HGLOBAL(handle.0),
                // Clipboard is open: failure to render advertised text is terminal
                // for this sequence; a later update will get a fresh budget.
                Err(_) => return Ok(ClipboardRead::Ignored),
            };
            let bytes = GlobalSize(handle);
            if !(2..=MAX_ITEM_BYTES + 2).contains(&bytes) || !bytes.is_multiple_of(2) {
                return Ok(ClipboardRead::Ignored);
            }
            let pointer = GlobalLock(handle) as *const u16;
            if pointer.is_null() {
                return Ok(ClipboardRead::Ignored);
            }
            let units = std::slice::from_raw_parts(pointer, bytes / 2);
            let length = units
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(units.len());
            let mut text = String::from_utf16_lossy(&units[..length]);
            let _ = GlobalUnlock(handle);
            let after = GetClipboardSequenceNumber();
            if !clipboard_sequence_unchanged(before, after) {
                text.zeroize();
                return Ok(ClipboardRead::Busy);
            }
            Ok(ClipboardRead::Text(after, text))
        })();
        let close = CloseClipboard();
        if close.is_err() {
            if let Ok(ClipboardRead::Text(_, text)) = &mut result {
                text.zeroize();
            }
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
    let item = service
        .inner
        .lock()
        .map_err(|_| "Clipboard history unavailable")?
        .items
        .iter()
        .find(|item| item.id == id)
        .cloned()
        .ok_or("Clipboard item not found")?;
    if media::missing(&item) {
        service.emit_changed();
        return Err("File no longer exists".into());
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::{
            Foundation::{GlobalFree, HANDLE},
            System::{
                DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
                Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            },
        };
        let (format, bytes) = if !item.paths.is_empty() {
            (15, Zeroizing::new(media::file_list(&item.paths)?))
        } else if let Some(image) = &item.image {
            (17, Zeroizing::new(image.as_ref().clone()))
        } else {
            if item.kind == "image" {
                return Err("Image unavailable".into());
            }
            (
                13,
                Zeroizing::new(
                    item.text
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .flat_map(u16::to_le_bytes)
                        .collect::<Vec<_>>(),
                ),
            )
        };
        let owner = window.hwnd().map_err(|_| "Clipboard window unavailable")?;
        let memory =
            GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|_| "Clipboard allocation failed")?;
        let ptr = GlobalLock(memory) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(memory);
            return Err("Clipboard allocation failed".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(memory);
        if OpenClipboard(windows::Win32::Foundation::HWND(owner.0 as _)).is_err() {
            let _ = GlobalFree(memory);
            return Err("Clipboard is busy".into());
        }
        let result =
            EmptyClipboard().and_then(|_| SetClipboardData(format, HANDLE(memory.0)).map(|_| ()));
        let legacy_result = if result.is_ok() {
            item.image
                .as_ref()
                .map_or(Ok(()), |image| media::copy_legacy(image))
        } else {
            Ok(())
        };
        let _ = CloseClipboard();
        if result.is_err() {
            let _ = GlobalFree(memory);
            return Err("Could not copy clipboard item".into());
        }
        legacy_result?;
        service
            .last_sequence
            .store(current_clipboard_sequence(), Ordering::Release);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = item;
        Err("Windows clipboard required".into())
    }
}

#[cfg(windows)]
fn current_clipboard_sequence() -> u32 {
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    unsafe { GetClipboardSequenceNumber() }
}

#[cfg(not(windows))]
fn read_clipboard_text() -> Result<ClipboardRead, String> {
    Ok(ClipboardRead::Ignored)
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
        let mut item = ClipboardHistoryItem::new(text.into());
        item.pinned = pinned;
        item.created_at_ms = 0;
        item
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

    #[test]
    fn privacy_markers_exclude_cooperating_sensitive_clipboards() {
        assert!(!should_skip_clipboard_item(false, false, false, false));
        assert!(should_skip_clipboard_item(true, false, false, false));
        assert!(should_skip_clipboard_item(false, true, false, false));
        assert!(should_skip_clipboard_item(false, false, true, false));
        assert!(should_skip_clipboard_item(false, false, false, true));
    }

    #[test]
    fn clipboard_change_during_native_read_discards_the_snapshot() {
        assert!(clipboard_sequence_unchanged(12, 12));
        assert!(!clipboard_sequence_unchanged(12, 13));
    }

    #[test]
    fn stale_read_is_rejected_after_disable_and_reenable() {
        let state = Arc::new(Mutex::new(StateInner {
            enabled: true,
            persist_on_exit: false,
            generation: 0,
            items: VecDeque::new(),
        }));
        let captured_generation = state.lock().unwrap().generation;
        let rendezvous = Arc::new(std::sync::Barrier::new(2));
        let worker_state = state.clone();
        let worker_rendezvous = rendezvous.clone();
        let worker = std::thread::spawn(move || {
            worker_rendezvous.wait();
            worker_rendezvous.wait();
            let state = worker_state.lock().unwrap();
            accept_clipboard_generation(state.enabled, state.generation, captured_generation)
        });

        rendezvous.wait();
        {
            let mut state = state.lock().unwrap();
            update_enabled(&mut state, false);
            update_enabled(&mut state, true);
        }
        rendezvous.wait();

        assert!(!worker.join().unwrap());
        let state = state.lock().unwrap();
        assert!(state.items.is_empty());
        assert_eq!(state.generation, 2);
    }

    #[test]
    fn history_input_and_persisted_schema_are_bounded() {
        assert!(!accepts_clipboard_text("  \n"));
        assert!(!accepts_clipboard_text(&"x".repeat(MAX_ITEM_BYTES + 1)));
        assert!(!valid_persisted_items(&PersistedItems {
            schema_version: 2,
            items: vec![],
        }));
        assert!(serde_json::from_str::<PersistedItems>(
            r#"{"schemaVersion":1,"items":[],"futureField":true}"#
        )
        .is_err());
    }

    #[test]
    fn bounded_storage_read_rejects_oversized_files_before_parsing() {
        let dir = std::env::temp_dir().join(format!("tuck-clipboard-bound-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("items.json");
        std::fs::write(&path, b"x".repeat(17)).unwrap();
        let error = read_bounded(&path, 16).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        std::fs::write(&path, b"small").unwrap();
        assert_eq!(read_bounded(&path, 16).unwrap(), b"small");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    #[test]
    fn busy_sequence_exhausts_budget_and_duplicate_notifications_do_not_renew_it() {
        let mut retry = CaptureRetry::default();
        retry.notified(123, 0);
        let mut total_delay = std::time::Duration::ZERO;
        for attempt in 0..CAPTURE_ATTEMPTS {
            assert!(retry.delay.is_some());
            assert!(!retry.notified(123, 0));
            retry.observed(&Ok(ClipboardRead::Busy));
            if attempt + 1 < CAPTURE_ATTEMPTS {
                total_delay += retry.delay.unwrap();
            }
        }
        assert_eq!(total_delay, std::time::Duration::from_millis(5240));
        assert!(retry.delay.is_none());
        retry.notified(123, 0);
        assert!(retry.delay.is_none());
    }
    #[test]
    fn exhausted_worker_parks_until_later_change_then_captures_and_parks_again() {
        let mut retry = CaptureRetry::default();
        retry.notified(123, 0);
        for _ in 0..CAPTURE_ATTEMPTS {
            retry.observed(&Ok(ClipboardRead::Busy));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            ready_tx.send(()).unwrap();
            retry.wait(&rx).unwrap();
            retry.notified(124, 0);
            assert!(retry.delay.is_some());
            let result = Ok(ClipboardRead::Text(124, "later clipboard change".into()));
            retry.observed(&result);
            read_tx.send(result).unwrap();
            assert!(retry.delay.is_none());
            assert!(retry.wait(&rx).is_err());
        });
        ready_rx.recv().unwrap();
        assert!(matches!(
            read_rx.recv_timeout(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        tx.send(()).unwrap();
        assert_eq!(
            read_rx.recv().unwrap(),
            Ok(ClipboardRead::Text(124, "later clipboard change".into()))
        );
        drop(tx);
        worker.join().unwrap();
    }
    #[test]
    fn terminal_results_and_disable_park_and_reenable_gets_fresh_budget() {
        for result in [Ok(ClipboardRead::Ignored), Err("terminal error".into())] {
            let mut retry = CaptureRetry::default();
            assert!(retry.notified(123, 0));
            retry.observed(&result);
            assert!(retry.delay.is_none());
            retry.notified(123, 1);
            assert_eq!(retry.remaining, CAPTURE_ATTEMPTS);
            retry.completed();
            assert!(retry.delay.is_none());
        }
    }
}

#[cfg(test)]
mod visual_tests {
    use super::*;
    fn image() -> ClipboardHistoryItem {
        let mut dib = vec![0; 44];
        dib[..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        media::prepare(media::Capture::Dib(dib)).unwrap()
    }
    #[test]
    fn legacy_history_loads_with_text_defaults_and_current_schema_round_trips() {
        let id = Uuid::new_v4();
        let json = format!(
            r#"{{"schemaVersion":1,"items":[{{"id":"{id}","text":"Old fixture","createdAtMs":1,"pinned":true}}]}}"#
        );
        let stored: PersistedItems = serde_json::from_str(&json).unwrap();
        assert!(valid_persisted_items(&stored));
        assert_eq!(stored.items[0].kind, "text");
        assert_eq!(
            serde_json::from_slice::<PersistedItems>(&serde_json::to_vec(&stored).unwrap())
                .unwrap()
                .items,
            stored.items
        );
    }
    #[test]
    fn image_pressure_evicts_oldest_unpinned_images_and_keeps_pins_and_text() {
        let payload = Arc::new(vec![0; media::MAX_DECODED]);
        let mut items = VecDeque::new();
        let mut pinned = image();
        pinned.pinned = true;
        pinned.image = Some(payload.clone());
        let id = pinned.id.clone();
        items.push_back(pinned);
        for _ in 0..8 {
            let mut next = image();
            next.image = Some(payload.clone());
            items.push_front(next);
        }
        let oldest_unpinned = items[7].id.clone();
        items.push_front(ClipboardHistoryItem::new("Keep text".into()));
        evict_unpinned(&mut items);
        assert!(items.iter().any(|item| item.id == id));
        assert!(items.iter().any(|item| item.kind == "text"));
        assert!(!items.iter().any(|item| item.id == oldest_unpinned));
        assert!(items.iter().map(media::bytes).sum::<usize>() <= media::MAX_IMAGES);
    }
    #[test]
    fn save_remove_clear_and_disable_delete_only_owned_media() {
        let dir = std::env::temp_dir().join(format!("tuck-visual-test-{}", Uuid::new_v4()));
        let mut pinned = image();
        pinned.pinned = true;
        let other = image();
        // Merely capturing/preparing/snapshotting has no filesystem writes.
        assert!(!dir.exists());
        let mut state = StateInner {
            enabled: true,
            persist_on_exit: false,
            generation: 0,
            items: VecDeque::from([pinned.clone(), other.clone()]),
        };
        assert!(!dir.exists());
        save_history(&dir, &[pinned.clone(), other.clone()]).unwrap();
        assert!(dir
            .join("images")
            .join(format!("{}.dib", other.id))
            .exists());
        save_history(&dir, std::slice::from_ref(&pinned)).unwrap();
        assert!(!dir
            .join("images")
            .join(format!("{}.dib", other.id))
            .exists());
        let mut stored: PersistedItems = serde_json::from_slice(
            &read_bounded(&dir.join("items.json"), MAX_STORE_BYTES).unwrap(),
        )
        .unwrap();
        restore_images(&dir, &mut stored.items).unwrap();
        assert_eq!(stored.items[0].image, pinned.image);
        assert!(stored.items[0].pinned);
        let temp = dir.join(format!("items.json.{}.tmp", Uuid::new_v4()));
        std::fs::write(&temp, b"Synthetic private fixture").unwrap();
        remove_history(&dir).unwrap();
        assert!(!temp.exists());
        update_enabled(&mut state, false);
        assert!(state.items.is_empty());
        assert!(!dir.join("items.json").exists());
        assert_eq!(std::fs::read_dir(dir.join("images")).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn failed_index_replacement_keeps_previous_image_and_removes_new_orphans() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("tuck-visual-atomic-{}", Uuid::new_v4()));
        let previous = image();
        save_history(&dir, std::slice::from_ref(&previous)).unwrap();
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(dir.join("items.json"))
            .unwrap();
        let next = image();
        assert!(save_history(&dir, std::slice::from_ref(&next)).is_err());
        assert!(dir
            .join("images")
            .join(format!("{}.dib", previous.id))
            .exists());
        assert!(!dir.join("images").join(format!("{}.dib", next.id)).exists());
        drop(locked);
        remove_history(&dir).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn categories_and_links_are_unambiguous() {
        assert_eq!(
            ClipboardHistoryItem::new("HTTPS://example.com/a".into()).kind,
            "link"
        );
        for text in ["https://", "https://a.test more", "file:///C:/a", "hello"] {
            assert_eq!(ClipboardHistoryItem::new(text.into()).kind, "text");
        }
    }
}
