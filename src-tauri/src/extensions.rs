//! Opt-in Windows v1 services. All external I/O happens outside the snapshot mutex.
use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
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
    pub calendar: bool,
    pub notifications: bool,
    pub radios: bool,
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
pub struct Call {
    pub title: String,
    pub started_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub join_url: Option<String>,
    pub all_day: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfItem {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadioInfo {
    pub kind: String,
    pub name: String,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub settings: Settings,
    pub downloads: Vec<Download>,
    pub call: Option<Call>,
    pub calendar: Vec<CalendarEvent>,
    pub shelf: Vec<ShelfItem>,
    pub radios: Vec<RadioInfo>,
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
static CALENDAR_OPERATION: Mutex<()> = Mutex::new(());
static SETTINGS_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
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
pub fn validate_http_url(raw: &str) -> Option<String> {
    if raw.chars().any(char::is_control) {
        return None;
    }
    let raw = raw
        .trim()
        .trim_matches(|c| c == '<' || c == '>' || c == '"');
    if raw.chars().any(char::is_control) {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.into())
}
fn date(value: &str) -> Result<(i64, bool), String> {
    if !value.is_ascii() {
        return Err("Invalid calendar date".into());
    }
    if value.len() == 8 {
        let date =
            NaiveDate::parse_from_str(value, "%Y%m%d").map_err(|_| "Invalid calendar date")?;
        let time = Local
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).ok_or("Invalid calendar date")?)
            .single()
            .ok_or("Ambiguous local calendar date")?;
        return Ok((time.timestamp_millis(), true));
    }
    let utc = value.ends_with('Z');
    let raw = if utc {
        &value[..value.len() - 1]
    } else {
        value
    };
    if raw.len() != 15 {
        return Err("Invalid calendar timestamp".into());
    }
    let time = NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%S")
        .map_err(|_| "Invalid calendar timestamp")?;
    let ms = if utc {
        time.and_utc().timestamp_millis()
    } else {
        Local
            .from_local_datetime(&time)
            .single()
            .ok_or("Ambiguous local calendar timestamp")?
            .timestamp_millis()
    };
    Ok((ms, false))
}
pub fn parse_ics(text: &str) -> Result<Vec<CalendarEvent>, String> {
    if text.len() > 2 * 1024 * 1024 {
        return Err("Calendar exceeds 2 MB".into());
    }
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(&line[1..]);
            }
        } else {
            lines.push(line.trim_end_matches('\r').into());
        }
    }
    if !lines.iter().any(|line| line == "BEGIN:VCALENDAR") {
        return Err("Choose an iCalendar file".into());
    }
    let mut events = Vec::new();
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut inside = false;
    for line in lines {
        if line == "BEGIN:VEVENT" {
            if inside {
                return Err("Nested calendar event".into());
            }
            fields.clear();
            inside = true;
            continue;
        }
        if line == "END:VEVENT" && inside {
            inside = false;
            if fields.get("STATUS").map(String::as_str) == Some("CANCELLED") {
                continue;
            }
            let (start, all_day) = date(fields.get("DTSTART").ok_or("Event has no start time")?)?;
            let end = match fields.get("DTEND") {
                Some(value) => date(value)?.0,
                None => start + if all_day { 86_400_000 } else { 1_800_000 },
            };
            if end <= start {
                return Err("Event ends before it starts".into());
            }
            let join_url = ["URL", "LOCATION", "DESCRIPTION"]
                .iter()
                .filter_map(|key| fields.get(*key))
                .flat_map(|value| value.split_whitespace())
                .find_map(|word| validate_http_url(word.trim_end_matches([',', ')', ';'])));
            let uid = fields
                .get("UID")
                .cloned()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            events.push(CalendarEvent {
                id: format!("{uid}:{start}"),
                title: fields
                    .get("SUMMARY")
                    .cloned()
                    .unwrap_or_else(|| "Calendar event".into()),
                starts_at: start,
                ends_at: end,
                join_url,
                all_day,
            });
            if events.len() > 2000 {
                return Err("Calendar exceeds 2,000 events".into());
            }
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let upper = key.to_uppercase();
            let property = upper.split(';').next().unwrap_or("");
            if upper.contains("TZID=")
                || ["RRULE", "RDATE", "EXDATE", "RECURRENCE-ID"].contains(&property)
            {
                return Err("Recurring events and TZID calendars are not supported yet. Export individual events in UTC.".into());
            }
            fields.insert(
                property.to_owned(),
                value
                    .replace("\\n", "\n")
                    .replace("\\N", "\n")
                    .replace("\\,", ",")
                    .replace("\\;", ";"),
            );
        }
    }
    if inside {
        return Err("Incomplete calendar event".into());
    }
    events.sort_by_key(|event| event.starts_at);
    Ok(events)
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
        let _operation = CALENDAR_OPERATION
            .lock()
            .map_err(|_| "Settings operation unavailable")?;
        write_local(
            "extensions.json",
            &serde_json::to_vec(&settings).map_err(|_| "Invalid settings")?,
        )?;
        let calendar = if settings.calendar {
            STORAGE
                .get()
                .and_then(|dir| bounded_read(&dir.join("calendar.ics"), 2 * 1024 * 1024).ok())
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .and_then(|text| parse_ics(&text).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut state = state().lock().unwrap_or_else(|p| p.into_inner());
        state.settings = settings.clone();
        SETTINGS_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        state.calendar = calendar;
        if !settings.downloads {
            state.downloads.clear();
        }
        if !settings.radios {
            state.radios.clear();
        }
        if !settings.notifications {
            state.notification_status = "Disabled".into();
        }
        Ok(settings)
    })
    .await
    .map_err(|_| "Settings worker failed".to_string())?
}
#[tauri::command]
pub fn call_set_active(window: WebviewWindow, active: bool) -> Result<(), String> {
    ensure(&window, false)?;
    state().lock().unwrap_or_else(|p| p.into_inner()).call = if active {
        Some(Call {
            title: "Manual call timer".into(),
            started_at: Utc::now().timestamp_millis(),
        })
    } else {
        None
    };
    Ok(())
}
#[tauri::command]
pub async fn calendar_import(window: WebviewWindow, text: String) -> Result<usize, String> {
    ensure(&window, true)?;
    tauri::async_runtime::spawn_blocking(move || commit_calendar(&text))
        .await
        .map_err(|_| "Calendar worker failed".to_string())?
}
fn commit_calendar(text: &str) -> Result<usize, String> {
    let _operation = CALENDAR_OPERATION
        .lock()
        .map_err(|_| "Calendar operation unavailable")?;
    let events = parse_ics(text)?;
    let count = events.len();
    write_local("calendar.ics", text.as_bytes())?;
    let mut state = state().lock().unwrap_or_else(|p| p.into_inner());
    if state.settings.calendar {
        state.calendar = events;
    }
    Ok(count)
}
#[tauri::command]
pub fn calendar_join(window: WebviewWindow, id: String) -> Result<(), String> {
    ensure(&window, false)?;
    let event = snapshot()
        .calendar
        .into_iter()
        .find(|event| event.id == id)
        .ok_or("Calendar event unavailable")?;
    let url = event
        .join_url
        .as_deref()
        .and_then(validate_http_url)
        .ok_or("No valid meeting link")?;
    window
        .opener()
        .open_url(url, None::<&str>)
        .map_err(|_| "Could not open meeting link".into())
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
#[cfg(windows)]
fn read_radios() -> Result<Vec<RadioInfo>, String> {
    use windows::Devices::Radios::{Radio, RadioKind, RadioState};
    let _apartment = Apartment::enter()?;
    let radios = Radio::GetRadiosAsync()
        .and_then(|op| op.get())
        .map_err(|_| "Radio enumeration unavailable")?;
    let mut out = Vec::new();
    for radio in radios {
        let kind = radio.Kind().map_err(|_| "Radio kind unavailable")?;
        let kind = if kind == RadioKind::WiFi {
            "wifi"
        } else if kind == RadioKind::Bluetooth {
            "bluetooth"
        } else {
            continue;
        };
        let state = radio.State().map_err(|_| "Radio state unavailable")?;
        out.push(RadioInfo {
            kind: kind.into(),
            name: radio
                .Name()
                .map(|s| s.to_string())
                .unwrap_or_else(|_| kind.into()),
            state: if state == RadioState::On {
                "On"
            } else if state == RadioState::Off {
                "Off"
            } else if state == RadioState::Disabled {
                "Disabled"
            } else {
                "Unknown"
            }
            .into(),
        });
    }
    Ok(out)
}
#[tauri::command]
pub async fn radio_set(window: WebviewWindow, kind: String, enabled: bool) -> Result<(), String> {
    ensure(&window, false)?;
    if !snapshot().settings.radios {
        return Err("Enable radio controls in settings first".into());
    }
    if !["wifi", "bluetooth"].contains(&kind.as_str()) {
        return Err("Unknown radio".into());
    }
    let epoch = SETTINGS_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
    #[cfg(windows)]
    {
        use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
        let (tx, rx) = std::sync::mpsc::channel();
        window
            .run_on_main_thread(move || {
                let _ = tx.send(Radio::RequestAccessAsync());
            })
            .map_err(|_| "Permission dispatch failed")?;
        tauri::async_runtime::spawn_blocking(move || {
            let _apartment = Apartment::enter()?;
            let op = rx
                .recv()
                .map_err(|_| "Permission request failed")?
                .map_err(|_| "Radio access unavailable")?;
            let access = op.get().map_err(|_| "Radio permission failed")?;
            if access != RadioAccessStatus::Allowed {
                return Err("Windows denied radio control".into());
            }
            let radios = Radio::GetRadiosAsync()
                .and_then(|op| op.get())
                .map_err(|_| "Radio enumeration failed")?;
            let target = if kind == "wifi" {
                RadioKind::WiFi
            } else {
                RadioKind::Bluetooth
            };
            let mut found = false;
            for radio in radios {
                if radio.Kind().ok() == Some(target) {
                    if !snapshot().settings.radios
                        || SETTINGS_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != epoch
                    {
                        return Err("Radio opt-in changed; action cancelled".into());
                    }
                    found = true;
                    let result = radio
                        .SetStateAsync(if enabled {
                            RadioState::On
                        } else {
                            RadioState::Off
                        })
                        .and_then(|op| op.get())
                        .map_err(|_| "Radio change failed")?;
                    if result != RadioAccessStatus::Allowed {
                        return Err("Windows denied radio change".into());
                    }
                }
            }
            if !found {
                return Err("No matching radio hardware".into());
            }
            Ok(())
        })
        .await
        .map_err(|_| "Radio worker failed".to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = (kind, enabled);
        Err("Windows required".into())
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
                let mut state = state().lock().unwrap_or_else(|p| p.into_inner());
                state.settings = settings;
            }
        }
    }
    if snapshot().settings.calendar {
        if let Some(dir) = STORAGE.get() {
            if let Ok(bytes) = bounded_read(&dir.join("calendar.ics"), 2 * 1024 * 1024) {
                if let Ok(text) = String::from_utf8(bytes) {
                    if let Ok(events) = parse_ics(&text) {
                        state().lock().unwrap_or_else(|p| p.into_inner()).calendar = events;
                    }
                }
            }
        }
    }
    std::thread::spawn(move || {
        #[cfg(windows)]
        let _apartment = Apartment::enter();
        let mut history = HashMap::new();
        let mut reminded = HashSet::new();
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
            #[cfg(windows)]
            let radios = if settings.radios {
                read_radios().unwrap_or_else(|error| {
                    errors.push(error);
                    Vec::new()
                })
            } else {
                Vec::new()
            };
            #[cfg(not(windows))]
            let radios = Vec::new();
            let calendar = if settings.calendar {
                current.calendar.clone()
            } else {
                Vec::new()
            };
            let now = Utc::now().timestamp_millis();
            for event in &calendar {
                if !event.all_day
                    && (0..=300_000).contains(&(event.starts_at - now))
                    && reminded.insert(event.id.clone())
                    && snapshot().settings.calendar
                {
                    let _ = app.emit("calendar-reminder", event);
                }
            }
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
                    state.radios = radios;

                    state.notification_status = notification_status;
                    state.errors = errors;
                }
            }
            let _ = app.emit("extension-state", snapshot());

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
    fn calendar_rejects_misleading_inputs() {
        for text in ["BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART;TZID=America/New_York:20260101T120000\nEND:VEVENT", "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20261399T120000Z\nEND:VEVENT", "BEGIN:VCALENDAR\nBEGIN:VEVENT\nRRULE:FREQ=DAILY\nEND:VEVENT"]{assert!(parse_ics(text).is_err());}
    }
    #[test]
    fn calendar_unfolds_and_preserves_utc() {
        let events=parse_ics("BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nDTSTART:20261102T120000Z\nSUMMARY:Plan\n ning\nURL:https://example.com/meet\nEND:VEVENT\nEND:VCALENDAR").unwrap();
        assert_eq!(events[0].title, "Planning");
        assert_eq!(events[0].starts_at, 1793620800000);
        assert_eq!(
            events[0].join_url.as_deref(),
            Some("https://example.com/meet")
        );
    }
    #[test]
    fn urls_are_restricted() {
        assert!(validate_http_url("https://example.com/meet").is_some());
        for url in [
            "javascript:alert(1)",
            "file:///C:/a",
            "https://user:pass@example.com/",
            "https://example.com/\n",
        ] {
            assert!(validate_http_url(url).is_none());
        }
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
    #[test]
    fn concurrent_imports_keep_disk_and_memory_together() {
        let dir = std::env::temp_dir().join(format!("tuck-calendar-test-{}", uuid::Uuid::new_v4()));
        STORAGE
            .set(dir.clone())
            .expect("test-local calendar storage");
        state().lock().unwrap().settings.calendar = true;
        let workers=(0..2).map(|index|std::thread::spawn(move||{
            commit_calendar(&format!("BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:event-{index}\nDTSTART:20300101T120000Z\nSUMMARY:Import {index}\nEND:VEVENT\nEND:VCALENDAR")).unwrap();
        })).collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        let persisted =
            parse_ics(&std::fs::read_to_string(dir.join("calendar.ics")).unwrap()).unwrap();
        assert_eq!(persisted[0].id, snapshot().calendar[0].id);
        std::fs::remove_file(dir.join("calendar.ics")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
