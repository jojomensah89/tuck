//! Opt-in Windows v1 services. All external I/O happens outside the snapshot mutex.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

static SETTINGS_WAKE: Mutex<Option<std::sync::mpsc::Sender<()>>> = Mutex::new(None);

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub notifications: bool,
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
        if !settings.notifications {
            state.notification_status = "Disabled".into();
        }
        if let Some(wake) = SETTINGS_WAKE
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            let _ = wake.send(());
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
    let total = state.shelf.len();
    drop(state);
    // Each thumbnail is made once, in the background, as soon as its file arrives
    for item in &items {
        crate::shell_thumbs::prefetch(&crate::shell_thumbs::shelf_key(&item.id), &item.path);
    }
    #[cfg(debug_assertions)]
    if std::env::var_os("TUCK_DROP_TRACE").is_some() {
        eprintln!("drop-shelf added={} total={total}", items.len());
    }
    #[cfg(not(debug_assertions))]
    let _ = total;
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
    remove_items(id.as_deref());
    Ok(())
}
/// Takes items off the shelf (all of them for `None`) and drops their thumbnails.
/// The files themselves are never touched.
fn remove_items(id: Option<&str>) {
    let mut state = state().lock().unwrap_or_else(|p| p.into_inner());
    let removed = state
        .shelf
        .iter()
        .filter(|item| id.is_none_or(|id| item.id == id))
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    state.shelf.retain(|item| !removed.contains(&item.id));
    drop(state);
    for id in removed {
        crate::shell_thumbs::forget(&crate::shell_thumbs::shelf_key(&id));
    }
}
fn shelf_path(id: &str) -> Result<PathBuf, String> {
    snapshot()
        .shelf
        .into_iter()
        .find(|item| item.id == id)
        .map(|item| item.path)
        .ok_or("Shelf item unavailable".into())
}
/// Only a file that is on the shelf can be opened, and only by its shelf id: the
/// webview never supplies a path.
fn openable_path(id: &str) -> Result<PathBuf, String> {
    let path = shelf_path(id)?;
    if !path.is_file() {
        return Err("File no longer exists".into());
    }
    Ok(crate::shell_thumbs::shell_path(&path))
}
#[tauri::command]
pub fn shelf_open(window: WebviewWindow, id: String) -> Result<(), String> {
    ensure(&window, false)?;
    let path = openable_path(&id)?;
    window
        .opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|_| "Could not open file".into())
}
#[tauri::command]
pub async fn shelf_thumbnail(
    window: WebviewWindow,
    id: String,
) -> Result<crate::shell_thumbs::Thumb, String> {
    ensure(&window, false)?;
    let path = shelf_path(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::shell_thumbs::thumbnail(&crate::shell_thumbs::shelf_key(&id), &path)
    })
    .await
    .map_err(|_| "Thumbnail worker failed".to_string())
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
pub fn start(app: AppHandle) {
    let (wake, updates) = std::sync::mpsc::channel();
    *SETTINGS_WAKE.lock().unwrap_or_else(|p| p.into_inner()) = Some(wake);
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
        let mut seen_notifications: Option<HashSet<u32>> = None;
        loop {
            let current = snapshot();
            let settings = current.settings.clone();
            let mut errors = Vec::new();
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
            if settings.notifications {
                let _ = updates.recv_timeout(Duration::from_secs(1));
            } else if updates.recv().is_err() {
                break;
            }
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
        assert!(!Settings::default().notifications);
        assert_eq!(
            serde_json::from_str::<Settings>(r#"{"downloads":true}"#).unwrap(),
            Settings::default()
        );
    }
    #[test]
    fn snapshot_contains_only_remaining_extension_services() {
        let json = serde_json::to_value(Snapshot::default()).unwrap();
        let settings = json.get("settings").unwrap();
        assert!(settings.get("downloads").is_none());
        assert!(settings.get("notifications").is_some());
        assert!(settings.get("calendar").is_none());
        assert!(settings.get("radios").is_none());
        assert!(json.get("call").is_none());
        assert!(json.get("calendar").is_none());
        assert!(json.get("radios").is_none());
    }

    #[test]
    fn open_is_restricted_to_shelf_items() {
        let path = std::env::temp_dir().join(format!("tuck-open-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"sample").unwrap();
        let added = add_paths(vec![path.clone()]).unwrap();
        let id = added[0].id.clone();
        assert_eq!(
            openable_path(&id).unwrap(),
            crate::shell_thumbs::shell_path(&added[0].path)
        );
        // Anything that is not a shelf id is refused, including a path
        assert!(openable_path("not-a-shelf-id").is_err());
        assert!(openable_path(&path.to_string_lossy()).is_err());
        // A file that went away is reported, not handed to the shell
        std::fs::remove_file(&path).unwrap();
        assert_eq!(openable_path(&id).unwrap_err(), "File no longer exists");
        remove_items(Some(&id));
        assert!(openable_path(&id).is_err());
    }

    #[test]
    fn removing_items_drops_their_thumbnails_but_not_the_files() {
        let path = std::env::temp_dir().join(format!("tuck-thumb-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"sample").unwrap();
        let added = add_paths(vec![path.clone()]).unwrap();
        let key = crate::shell_thumbs::shelf_key(&added[0].id);
        // Adding registers the item, so its thumbnail is wanted and made once
        let first = crate::shell_thumbs::thumbnail(&key, &added[0].path);
        assert_eq!(crate::shell_thumbs::thumbnail(&key, &added[0].path), first);
        remove_items(Some(&added[0].id));
        assert_eq!(
            crate::shell_thumbs::thumbnail(&key, &added[0].path),
            crate::shell_thumbs::Thumb::None,
            "a removed item has no thumbnail"
        );
        assert!(path.exists());
        std::fs::remove_file(path).unwrap();
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
