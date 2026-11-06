//! Opt-in Windows v1 services. All external I/O happens outside the snapshot mutex.
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub downloads: bool,
    pub notifications: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub id: String,
    pub name: String,
    pub bytes: u64,
    pub bytes_per_second: u64,
    pub total_bytes: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfItem {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub settings: Settings,
    pub downloads: Vec<Download>,
    pub shelf: Vec<ShelfItem>,
    pub notification_status: String,
    pub errors: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub id: u32,
    pub title: String,
    pub body: String,
}
static STATE: OnceLock<Mutex<Snapshot>> = OnceLock::new();
static STORAGE: OnceLock<PathBuf> = OnceLock::new();
fn state() -> &'static Mutex<Snapshot> {
    STATE.get_or_init(|| Mutex::new(Snapshot::default()))
}
fn snapshot() -> Snapshot {
    state().lock().unwrap_or_else(|p| p.into_inner()).clone()
}
fn ensure(window: &WebviewWindow, settings_only: bool) -> Result<(), String> {
    if window.label() == "settings" || (!settings_only && window.label() == "main") {
        Ok(())
    } else {
        Err("This action is not allowed in this window".into())
    }
}
fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "Could not open local data")?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read local data")?;
    if bytes.len() as u64 > limit {
        Err("Local data exceeds size limit".into())
    } else {
        Ok(bytes)
    }
}
fn write_local(name: &str, bytes: &[u8]) -> Result<(), String> {
    let dir = STORAGE.get().ok_or("Local storage unavailable")?;
    std::fs::create_dir_all(dir).map_err(|_| "Could not create local storage")?;
    let tmp = dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes).map_err(|_| "Could not save local data")?;
    if std::fs::rename(&tmp, dir.join(name)).is_err() {
        let _ = std::fs::remove_file(tmp);
        return Err("Could not replace local data".into());
    }
    Ok(())
}
#[tauri::command]
pub fn extension_snapshot() -> Snapshot {
    snapshot()
}
#[tauri::command]
pub async fn extension_settings(
    window: WebviewWindow,
    settings: Settings,
) -> Result<Settings, String> {
    ensure(&window, true)?;
    tauri::async_runtime::spawn_blocking(move || {
        write_local(
            "extensions.json",
            &serde_json::to_vec(&settings).map_err(|_| "Invalid settings")?,
        )?;
        let mut state = state().lock().unwrap_or_else(|p| p.into_inner());
        state.settings = settings.clone();
        if !settings.downloads {
            state.downloads.clear();
        }
        if !settings.notifications {
            state.notification_status = "Disabled".into();
        }
        Ok(settings)
    })
    .await
    .map_err(|_| "Settings worker failed".to_string())?
}
fn add_paths(paths: Vec<PathBuf>) -> Result<Vec<ShelfItem>, String> {
    if paths.len() > 50 {
        return Err("Add at most 50 files at once".into());
    }
    let mut items = Vec::new();
    for path in paths {
        let path = path.canonicalize().map_err(|_| "File unavailable")?;
        let meta = path.metadata().map_err(|_| "File unavailable")?;
        if !meta.is_file() {
            return Err("Shelf accepts files only".into());
        }
        items.push(ShelfItem {
            id: uuid::Uuid::new_v4().to_string(),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            path,
            bytes: meta.len(),
        });
    }
    let mut state = state().lock().unwrap_or_else(|p| p.into_inner());
    if state.shelf.len() + items.len() > 100 {
        return Err("Shelf holds at most 100 files".into());
    }
    state.shelf.extend(items.clone());
    Ok(items)
}
#[tauri::command]
pub async fn shelf_add(
    window: WebviewWindow,
    paths: Vec<PathBuf>,
) -> Result<Vec<ShelfItem>, String> {
    ensure(&window, false)?;
    tauri::async_runtime::spawn_blocking(move || add_paths(paths))
        .await
        .map_err(|_| "Shelf worker failed".to_string())?
}
#[tauri::command]
pub async fn shelf_pick(window: WebviewWindow) -> Result<(), String> {
    ensure(&window, false)?;
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(files) = window.dialog().file().blocking_pick_files() {
            let paths = files
                .into_iter()
                .map(|file| file.into_path().map_err(|_| "File path unavailable"))
                .collect::<Result<Vec<_>, _>>()?;
            add_paths(paths)?;
        }
        Ok(())
    })
    .await
    .map_err(|_| "File picker failed".to_string())?
}
#[tauri::command]
pub fn shelf_remove(window: WebviewWindow, id: Option<String>) -> Result<(), String> {
    ensure(&window, false)?;
    let mut state = state().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(id) = id {
        state.shelf.retain(|item| item.id != id);
    } else {
        state.shelf.clear();
    }
    Ok(())
}
fn shelf_path(id: &str) -> Result<PathBuf, String> {
    snapshot()
        .shelf
        .into_iter()
        .find(|item| item.id == id)
        .map(|item| item.path)
        .ok_or("Shelf item unavailable".into())
}
#[tauri::command]
pub fn shelf_reveal(window: WebviewWindow, id: String) -> Result<(), String> {
    ensure(&window, false)?;
    window
        .opener()
        .reveal_item_in_dir(shelf_path(&id)?)
        .map_err(|_| "Could not reveal file".into())
}
#[tauri::command]
pub async fn shelf_drag(window: WebviewWindow, id: String) -> Result<(), String> {
    ensure(&window, false)?;
    let path = shelf_path(&id)?;
    #[cfg(windows)]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let drag_window = window.clone();
        window
            .run_on_main_thread(move || {
                let result = drag::start_drag(
                    &drag_window,
                    drag::DragItem::Files(vec![path]),
                    drag::Image::Raw(include_bytes!("../icons/32x32.png").to_vec()),
                    |_, _| {},
                    drag::Options::default(),
                )
                .map_err(|_| "Could not start native file drag".to_string());
                let _ = tx.send(result);
            })
            .map_err(|_| "Drag dispatch failed")?;
        tauri::async_runtime::spawn_blocking(move || {
            rx.recv().map_err(|_| "Drag worker failed".to_string())?
        })
        .await
        .map_err(|_| "Drag worker failed".to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("File dragging is supported on Windows".into())
    }
}
#[cfg(windows)]
struct Apartment;
#[cfg(windows)]
impl Apartment {
    fn enter() -> Result<Self, String> {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }
            .map_err(|_| "Windows service unavailable")?;
        Ok(Self)
    }
}
#[cfg(windows)]
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::System::Com::CoUninitialize();
        }
    }
}
#[tauri::command]
pub async fn notification_access(window: WebviewWindow) -> Result<String, String> {
    ensure(&window, true)?;
    #[cfg(windows)]
    {
        use windows::UI::Notifications::Management::UserNotificationListener;
        let (tx, rx) = std::sync::mpsc::channel();
        window
            .run_on_main_thread(move || {
                let _ = tx.send(
                    UserNotificationListener::Current()
                        .and_then(|listener| listener.RequestAccessAsync()),
                );
            })
            .map_err(|_| "Permission dispatch failed")?;
        let status=tauri::async_runtime::spawn_blocking(move|| {let _apartment=Apartment::enter()?;let operation=rx.recv().map_err(|_|"Permission request failed")?.map_err(|_|"Notification listener unavailable; Windows may require package identity and notification capability")?;operation.get().map(|status|format!("{status:?}")).map_err(|_|"Notification permission failed".to_string())}).await.map_err(|_|"Notification worker failed")??;
        state()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .notification_status = status.clone();
        Ok(status)
    }
    #[cfg(not(windows))]
    {
        Err("Windows notification listener unavailable".into())
    }
}
#[cfg(windows)]
fn notifications() -> Result<(String, Vec<Preview>), String> {
    use windows::{
        core::HSTRING,
        UI::Notifications::{
            Management::{UserNotificationListener, UserNotificationListenerAccessStatus},
            NotificationKinds,
        },
    };
    let listener = UserNotificationListener::Current().map_err(|_| {
        "Notification listener unavailable: package identity/capability may be required"
    })?;
    let access = listener
        .GetAccessStatus()
        .map_err(|_| "Notification access unavailable")?;
    if access != UserNotificationListenerAccessStatus::Allowed {
        return Ok((format!("{access:?}"), Vec::new()));
    }
    let items = listener
        .GetNotificationsAsync(NotificationKinds::Toast)
        .and_then(|op| op.get())
        .map_err(|_| "Notification retrieval failed")?;
    let mut previews = Vec::new();
    for item in items.into_iter().take(100) {
        let content = (|| -> windows::core::Result<Preview> {
            let binding = item
                .Notification()?
                .Visual()?
                .GetBinding(&HSTRING::from("ToastGeneric"))?;
            let texts = binding.GetTextElements()?;
            let mut lines = Vec::new();
            for text in texts {
                lines.push(text.Text()?.to_string());
            }
            Ok(Preview {
                id: item.Id()?,
                title: lines
                    .first()
                    .cloned()
                    .unwrap_or_default()
                    .chars()
                    .take(120)
                    .collect(),
                body: lines
                    .into_iter()
                    .skip(1)
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(300)
                    .collect(),
            })
        })();
        if let Ok(preview) = content {
            previews.push(preview);
        }
    }
    Ok(("Allowed".into(), previews))
}
fn downloads_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use windows::Win32::{
            System::Com::CoTaskMemFree,
            UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath, KF_FLAG_DEFAULT},
        };
        unsafe {
            let path = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None).ok()?;
            let owned = path.to_string().ok().map(PathBuf::from);
            CoTaskMemFree(Some(path.0 as _));
            owned
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}
fn scan_downloads(
    dir: &Path,
    history: &mut HashMap<PathBuf, (u64, Instant)>,
) -> Result<Vec<Download>, String> {
    let mut current = HashSet::new();
    let mut downloads = Vec::new();
    let now = Instant::now();
    for entry in std::fs::read_dir(dir)
        .map_err(|_| "Downloads directory unavailable")?
        .take(5000)
        .flatten()
    {
        let path = entry.path();
        let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !["crdownload", "part", "download"].contains(&extension) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(meta) if meta.is_file() => meta,
            _ => continue,
        };
        let bytes = meta.len();
        let rate = history
            .get(&path)
            .map(|(old, then)| {
                ((bytes.saturating_sub(*old)) as f64
                    / now.duration_since(*then).as_secs_f64().max(0.001)) as u64
            })
            .unwrap_or(0);
        history.insert(path.clone(), (bytes, now));
        current.insert(path.clone());
        downloads.push(Download {
            id: path.to_string_lossy().into(),
            name: entry.file_name().to_string_lossy().into(),
            bytes,
            bytes_per_second: rate,
            total_bytes: None,
        });
    }
    history.retain(|path, _| current.contains(path));
    Ok(downloads)
}
pub fn start(app: AppHandle) {
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = STORAGE.set(dir.clone());
        if let Ok(bytes) = bounded_read(&dir.join("extensions.json"), 16384) {
            if let Ok(settings) = serde_json::from_slice::<Settings>(&bytes) {
                state().lock().unwrap_or_else(|p| p.into_inner()).settings = settings;
            }
        }
    }
    std::thread::spawn(move || {
        let mut emitted = None;
        #[cfg(windows)]
        let _apartment = Apartment::enter();
        let mut history = HashMap::new();
        let mut seen_notifications: Option<HashSet<u32>> = None;
        loop {
            let current = snapshot();
            let settings = current.settings.clone();
            let mut errors = Vec::new();
            let downloads = if settings.downloads {
                downloads_dir()
                    .ok_or("Downloads directory unavailable".to_string())
                    .and_then(|dir| scan_downloads(&dir, &mut history))
                    .unwrap_or_else(|error| {
                        errors.push(error);
                        Vec::new()
                    })
            } else {
                history.clear();
                Vec::new()
            };
            let mut notification_status = if settings.notifications {
                current.notification_status.clone()
            } else {
                "Disabled".into()
            };
            #[cfg(windows)]
            if settings.notifications {
                match notifications() {
                    Ok((status, previews)) => {
                        notification_status = status;
                        let ids = previews
                            .iter()
                            .map(|preview| preview.id)
                            .collect::<HashSet<_>>();
                        if let Some(seen) = &seen_notifications {
                            for preview in previews {
                                if !seen.contains(&preview.id) && snapshot().settings.notifications
                                {
                                    let _ = app.emit("notification-preview", preview);
                                }
                            }
                        }
                        seen_notifications = Some(ids);
                    }
                    Err(error) => {
                        notification_status = error.clone();
                        errors.push(error);
                    }
                }
            } else {
                seen_notifications = None;
            }
            {
                let mut state = state().lock().unwrap_or_else(|p| p.into_inner());
                if state.settings == settings {
                    state.downloads = downloads;
                    state.notification_status = notification_status;
                    state.errors = errors;
                }
            }
            let current = snapshot();
            let serialized = serde_json::to_value(&current).ok();
            if serialized != emitted {
                let _ = app.emit("extension-state", current);
                emitted = serialized;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_opt_in() {
        assert_eq!(
            Settings::default(),
            serde_json::from_str::<Settings>("{}").unwrap()
        );
        assert!(!Settings::default().downloads);
    }
    #[test]
    fn snapshot_contains_only_remaining_extension_services() {
        let json = serde_json::to_value(Snapshot::default()).unwrap();
        let settings = json.get("settings").unwrap();
        assert!(settings.get("downloads").is_some());
        assert!(settings.get("notifications").is_some());
        assert!(settings.get("calendar").is_none());
        assert!(settings.get("radios").is_none());
        assert!(json.get("call").is_none());
        assert!(json.get("calendar").is_none());
        assert!(json.get("radios").is_none());
    }

    #[test]
    fn downloads_never_invent_total() {
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("x.part");
        std::fs::write(&path, b"1234").unwrap();
        let mut history = HashMap::new();
        let first = scan_downloads(&dir, &mut history).unwrap();
        assert_eq!(first[0].total_bytes, None);
        assert_eq!(first[0].bytes, 4);
        std::fs::remove_file(&path).unwrap();
        assert!(scan_downloads(&dir, &mut history).unwrap().is_empty());
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn removing_shelf_reference_does_not_delete_file() {
        let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&path, b"sample").unwrap();
        let added = add_paths(vec![path.clone()]).unwrap();
        assert!(shelf_path(&added[0].id).is_ok());
        state()
            .lock()
            .unwrap()
            .shelf
            .retain(|item| item.id != added[0].id);
        assert!(path.exists());
        assert!(shelf_path(&added[0].id).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
