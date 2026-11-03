use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex};
use tauri::{AppHandle, Emitter};

static SNAPSHOT: Mutex<Option<MediaState>> = Mutex::new(None);
static SELECTED_SESSION_ID: Mutex<Option<String>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(1);
#[cfg(windows)]
static SERVICE_TX: Mutex<Option<mpsc::Sender<MediaSignal>>> = Mutex::new(None);

fn default_true() -> bool {
    true
}

/// Media state contract matching frontend and guide.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaState {
    pub active: bool,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub artwork: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub source_app_id: Option<String>,
    #[serde(default = "default_true")]
    pub can_play_pause: bool,
    #[serde(default = "default_true")]
    pub can_skip_next: bool,
    #[serde(default = "default_true")]
    pub can_skip_prev: bool,
    #[serde(default)]
    pub can_seek: bool,
    #[serde(default)]
    pub generation: u64,
}

/// Session descriptor for player enumeration and selection.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaSessionInfo {
    pub id: String,
    pub source_app_id: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub is_selected: bool,
}

#[tauri::command]
pub fn media_now() -> MediaState {
    SNAPSHOT
        .lock()
        .ok()
        .and_then(|state| state.clone())
        .unwrap_or_default()
}

/// Clamps target seek position within valid range [0, duration].
pub fn clamp_seek_ms(ms: i64, duration_ms: Option<i64>) -> i64 {
    let non_neg = ms.max(0);
    match duration_ms {
        Some(dur) if dur > 0 => non_neg.min(dur),
        _ => non_neg,
    }
}

/// Converts 100-nanosecond WinRT ticks to milliseconds.
pub fn ticks_to_ms(ticks: i64) -> i64 {
    ticks / 10_000
}

/// Converts milliseconds to 100-nanosecond WinRT ticks with saturating arithmetic.
pub fn ms_to_ticks(ms: i64) -> i64 {
    ms.saturating_mul(10_000)
}

/// Maximum thumbnail stream bytes to read into memory (5 MB limit to prevent unbounded memory read).
const MAX_THUMBNAIL_BYTES: usize = 5 * 1024 * 1024;

#[cfg(windows)]
use base64::Engine;
#[cfg(windows)]
use windows::Storage::Streams::DataReader;
#[cfg(windows)]
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
#[cfg(windows)]
use windows::{
    Foundation::{EventRegistrationToken, TypedEventHandler},
    Media::Control::{
        CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession,
        GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionMediaProperties,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus, MediaPropertiesChangedEventArgs,
        PlaybackInfoChangedEventArgs, SessionsChangedEventArgs, TimelinePropertiesChangedEventArgs,
    },
};
#[cfg(windows)]
static SESSION_CATALOG: Mutex<Vec<(String, GlobalSystemMediaTransportControlsSession)>> =
    Mutex::new(Vec::new());
#[cfg(windows)]
fn same_session(
    left: &GlobalSystemMediaTransportControlsSession,
    right: &GlobalSystemMediaTransportControlsSession,
) -> bool {
    use windows::core::{IUnknown, Interface};
    match (left.cast::<IUnknown>(), right.cast::<IUnknown>()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(windows)]
fn current_session() -> Option<GlobalSystemMediaTransportControlsSession> {
    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .ok()?
        .get()
        .ok()?;

    let selected = SELECTED_SESSION_ID.lock().ok()?.clone();
    if let Some(target_id) = selected {
        let pinned = SESSION_CATALOG
            .lock()
            .ok()?
            .iter()
            .find(|(id, _)| *id == target_id)
            .map(|(_, session)| session.clone())?;
        let sessions = mgr.GetSessions().ok()?;
        for index in 0..sessions.Size().ok()? {
            if let Ok(current) = sessions.GetAt(index) {
                if same_session(&current, &pinned) {
                    return Some(current);
                }
            }
        }
        return None;
    }

    mgr.GetCurrentSession().ok()
}

#[cfg(windows)]
fn session_from_manager(
    mgr: &GlobalSystemMediaTransportControlsSessionManager,
) -> Option<GlobalSystemMediaTransportControlsSession> {
    let selected = SELECTED_SESSION_ID.lock().ok()?.clone();
    if let Some(target_id) = selected {
        let pinned = SESSION_CATALOG
            .lock()
            .ok()?
            .iter()
            .find(|(id, _)| *id == target_id)
            .map(|(_, session)| session.clone())?;
        let sessions = mgr.GetSessions().ok()?;
        for index in 0..sessions.Size().ok()? {
            if let Ok(current) = sessions.GetAt(index) {
                if same_session(&current, &pinned) {
                    return Some(current);
                }
            }
        }
        return None;
    }
    mgr.GetCurrentSession().ok()
}

#[cfg(windows)]
fn read_artwork(
    props: &GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Option<String> {
    let thumb_ref = props.Thumbnail().ok()?;
    let stream = thumb_ref.OpenReadAsync().ok()?.get().ok()?;
    let size = stream.Size().ok()? as usize;
    if size == 0 || size > MAX_THUMBNAIL_BYTES {
        return None;
    }

    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size as u32).ok()?.get().ok()?;
    let mut bytes = vec![0u8; size];
    reader.ReadBytes(&mut bytes).ok()?;

    let content_type = stream.ContentType().ok()?.to_string();
    let mime = if content_type.is_empty() {
        "image/jpeg"
    } else {
        &content_type
    };

    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{mime};base64,{b64}"))
}

#[cfg(windows)]
fn poll_session(
    session: &GlobalSystemMediaTransportControlsSession,
    last_track_id: &str,
    last_artwork: &Option<String>,
    art_attempts: &mut u8,
) -> Option<(MediaState, String, Option<String>)> {
    let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let title = props.Title().ok()?.to_string();
    let artist = props.Artist().ok()?.to_string();

    let playback_info = session.GetPlaybackInfo().ok()?;
    let playing = playback_info
        .PlaybackStatus()
        .map(|s| s == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
        .unwrap_or(false);

    let (position_ms, duration_ms) = session
        .GetTimelineProperties()
        .map(|tl| {
            let start = tl.StartTime().map(|d| d.Duration).unwrap_or(0);
            let end = tl.EndTime().map(|d| d.Duration).unwrap_or(0);
            let pos = tl.Position().map(|d| d.Duration).unwrap_or(0);
            (ticks_to_ms(pos - start), ticks_to_ms(end - start))
        })
        .unwrap_or((0, 0));

    let source = session
        .SourceAppUserModelId()
        .map(|id| id.to_string())
        .unwrap_or_default();
    let track_id = format!("{source}:{artist}:{title}:{duration_ms}");
    let changed = track_id != last_track_id;
    if changed {
        *art_attempts = 0;
    }

    let generation = if changed {
        GENERATION.fetch_add(1, Ordering::SeqCst) + 1
    } else {
        GENERATION.load(Ordering::SeqCst)
    };

    let controls = playback_info.Controls().ok();
    let can_play_pause = controls
        .as_ref()
        .map(|c| {
            c.IsPlayPauseToggleEnabled().unwrap_or(false)
                || c.IsPlayEnabled().unwrap_or(false)
                || c.IsPauseEnabled().unwrap_or(false)
        })
        .unwrap_or(true);
    let can_skip_next = controls
        .as_ref()
        .map(|c| c.IsNextEnabled().unwrap_or(false))
        .unwrap_or(true);
    let can_skip_prev = controls
        .as_ref()
        .map(|c| c.IsPreviousEnabled().unwrap_or(false))
        .unwrap_or(true);
    let can_seek = controls
        .as_ref()
        .map(|c| c.IsPlaybackPositionEnabled().unwrap_or(false))
        .unwrap_or(false)
        && duration_ms > 0;

    // Reuse cached artwork if track identity hasn't changed to avoid reading stream on every poll
    let artwork = if !changed && last_artwork.is_some() {
        last_artwork.clone()
    } else if *art_attempts < 10 {
        *art_attempts += 1;
        read_artwork(&props)
    } else {
        None
    };

    let state = MediaState {
        active: true,
        track_id: track_id.clone(),
        title,
        artist,
        playing,
        position_ms,
        duration_ms,
        artwork: artwork.clone(),
        session_id: Some(source.clone()),
        source_app_id: Some(source),
        can_play_pause,
        can_skip_next,
        can_skip_prev,
        can_seek,
        generation,
    };

    Some((state, track_id, artwork))
}

/// Enumerates all active Windows media sessions.
#[tauri::command]
pub async fn media_sessions() -> Result<Vec<MediaSessionInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(windows)]
        {
            let _apartment = Apartment::enter()?;
            let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
                .map_err(|e| format!("Failed to request session manager: {e}"))?
                .get()
                .map_err(|e| format!("Failed to get session manager: {e}"))?;

            let sessions = mgr
                .GetSessions()
                .map_err(|e| format!("Failed to enumerate sessions: {e}"))?;
            let count = sessions
                .Size()
                .map_err(|e| format!("Failed to get sessions count: {e}"))?;

            let selected = SELECTED_SESSION_ID.lock().ok().and_then(|g| g.clone());

            let mut list = Vec::new();
            let old_catalog = SESSION_CATALOG
                .lock()
                .map_err(|_| "Media session catalog unavailable")?
                .clone();
            let mut new_catalog = Vec::new();
            for i in 0..count {
                if let Ok(s) = sessions.GetAt(i) {
                    let source_app_id = s
                        .SourceAppUserModelId()
                        .map(|id| id.to_string())
                        .unwrap_or_default();
                    let session_id = old_catalog
                        .iter()
                        .find(|(_, previous)| same_session(previous, &s))
                        .map(|(id, _)| id.clone())
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    new_catalog.push((session_id.clone(), s.clone()));

                    let props = s
                        .TryGetMediaPropertiesAsync()
                        .ok()
                        .and_then(|op| op.get().ok());
                    let title = props
                        .as_ref()
                        .and_then(|p| p.Title().ok())
                        .map(|t| t.to_string())
                        .unwrap_or_default();
                    let artist = props
                        .as_ref()
                        .and_then(|p| p.Artist().ok())
                        .map(|a| a.to_string())
                        .unwrap_or_default();

                    let playback_info = s.GetPlaybackInfo().ok();
                    let playing = playback_info
                        .as_ref()
                        .and_then(|pi| pi.PlaybackStatus().ok())
                        .map(|st| {
                            st == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing
                        })
                        .unwrap_or(false);

                    let is_selected = selected
                        .as_deref()
                        .map(|sel| sel == session_id)
                        .unwrap_or(false);

                    list.push(MediaSessionInfo {
                        id: session_id,
                        source_app_id,
                        title,
                        artist,
                        playing,
                        is_selected,
                    });
                }
            }
            *SESSION_CATALOG
                .lock()
                .map_err(|_| "Media session catalog unavailable")? = new_catalog;
            if let Some(id) = selected {
                if !list.iter().any(|session| session.id == id) {
                    list.push(MediaSessionInfo {
                        id,
                        source_app_id: "Selected player unavailable".into(),
                        is_selected: true,
                        ..Default::default()
                    });
                }
            }
            Ok(list)
        }
        #[cfg(not(windows))]
        {
            Ok(Vec::new())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Selects a specific media session by ID, or clears selection to auto-track active player.
/// Restricted to main and settings windows.
#[tauri::command]
pub fn media_select_session(
    window: tauri::Window,
    session_id: Option<String>,
) -> Result<(), String> {
    {
        let w = &window;
        let label = w.label();
        if label != "main" && label != "settings" {
            return Err(
                "Access denied: only main and settings windows may select media sessions".into(),
            );
        }
    }

    #[cfg(windows)]
    if let Some(id) = session_id.as_ref().filter(|id| !id.trim().is_empty()) {
        if !SESSION_CATALOG
            .lock()
            .map_err(|_| "Media catalog unavailable")?
            .iter()
            .any(|(known, _)| known == id)
        {
            return Err("This media session is no longer available".into());
        }
    }
    select_session_id(session_id)
}

fn select_session_id(session_id: Option<String>) -> Result<(), String> {
    if session_id.as_ref().is_some_and(|id| id.len() > 1024) {
        return Err("Media session ID is too long".into());
    }
    let mut selected = SELECTED_SESSION_ID
        .lock()
        .map_err(|_| "Failed to acquire lock for selected session".to_string())?;

    match session_id {
        Some(ref id) if !id.trim().is_empty() => {
            *selected = Some(id.trim().to_string());
        }
        _ => {
            *selected = None;
        }
    }
    #[cfg(windows)]
    request(MediaSignal::ManagerChanged);
    Ok(())
}

#[cfg(windows)]
#[derive(Clone, Copy)]
enum MediaSignal {
    SessionChanged,
    ManagerChanged,
    RebindAll,
    Suspend,
    Stop,
}

#[cfg(windows)]
fn request(signal: MediaSignal) {
    if let Ok(guard) = SERVICE_TX.lock() {
        if let Some(sender) = guard.as_ref() {
            let _ = sender.send(signal);
        }
    }
}

#[cfg(windows)]
fn callback_send(sender: &mpsc::Sender<MediaSignal>, signal: MediaSignal) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = sender.send(signal);
    }));
    if result.is_err() {
        eprintln!("Media callback panicked");
    }
}

#[cfg(windows)]
struct ManagerSubscriptions {
    manager: GlobalSystemMediaTransportControlsSessionManager,
    sessions: EventRegistrationToken,
    current: EventRegistrationToken,
}

#[cfg(windows)]
impl ManagerSubscriptions {
    fn new(
        manager: &GlobalSystemMediaTransportControlsSessionManager,
        sender: &mpsc::Sender<MediaSignal>,
    ) -> windows::core::Result<Self> {
        let sessions_sender = sender.clone();
        let sessions = manager.SessionsChanged(&TypedEventHandler::<
            GlobalSystemMediaTransportControlsSessionManager,
            SessionsChangedEventArgs,
        >::new(move |_, _| {
            callback_send(&sessions_sender, MediaSignal::ManagerChanged);
            Ok(())
        }))?;
        let current_sender = sender.clone();
        let current = manager.CurrentSessionChanged(&TypedEventHandler::<
            GlobalSystemMediaTransportControlsSessionManager,
            CurrentSessionChangedEventArgs,
        >::new(move |_, _| {
            callback_send(&current_sender, MediaSignal::ManagerChanged);
            Ok(())
        }))?;
        Ok(Self {
            manager: manager.clone(),
            sessions,
            current,
        })
    }
}

#[cfg(windows)]
impl Drop for ManagerSubscriptions {
    fn drop(&mut self) {
        if let Err(error) = self.manager.RemoveSessionsChanged(self.sessions) {
            eprintln!("Media SessionsChanged cleanup: {error}");
        }
        if let Err(error) = self.manager.RemoveCurrentSessionChanged(self.current) {
            eprintln!("Media CurrentSessionChanged cleanup: {error}");
        }
    }
}

#[cfg(windows)]
struct SessionSubscriptions {
    session: GlobalSystemMediaTransportControlsSession,
    media: EventRegistrationToken,
    playback: EventRegistrationToken,
    timeline: EventRegistrationToken,
}

#[cfg(windows)]
impl SessionSubscriptions {
    fn new(
        session: &GlobalSystemMediaTransportControlsSession,
        sender: &mpsc::Sender<MediaSignal>,
    ) -> windows::core::Result<Self> {
        let media_sender = sender.clone();
        let media = session.MediaPropertiesChanged(&TypedEventHandler::<
            GlobalSystemMediaTransportControlsSession,
            MediaPropertiesChangedEventArgs,
        >::new(move |_, _| {
            callback_send(&media_sender, MediaSignal::SessionChanged);
            Ok(())
        }))?;
        let playback_sender = sender.clone();
        let playback = session.PlaybackInfoChanged(&TypedEventHandler::<
            GlobalSystemMediaTransportControlsSession,
            PlaybackInfoChangedEventArgs,
        >::new(move |_, _| {
            callback_send(&playback_sender, MediaSignal::SessionChanged);
            Ok(())
        }))?;
        let timeline_sender = sender.clone();
        let timeline = session.TimelinePropertiesChanged(&TypedEventHandler::<
            GlobalSystemMediaTransportControlsSession,
            TimelinePropertiesChangedEventArgs,
        >::new(move |_, _| {
            callback_send(&timeline_sender, MediaSignal::SessionChanged);
            Ok(())
        }))?;
        Ok(Self {
            session: session.clone(),
            media,
            playback,
            timeline,
        })
    }
}

#[cfg(windows)]
impl Drop for SessionSubscriptions {
    fn drop(&mut self) {
        if let Err(error) = self.session.RemoveMediaPropertiesChanged(self.media) {
            eprintln!("MediaPropertiesChanged cleanup: {error}");
        }
        if let Err(error) = self.session.RemovePlaybackInfoChanged(self.playback) {
            eprintln!("PlaybackInfoChanged cleanup: {error}");
        }
        if let Err(error) = self.session.RemoveTimelinePropertiesChanged(self.timeline) {
            eprintln!("TimelinePropertiesChanged cleanup: {error}");
        }
    }
}

#[cfg(windows)]
fn emit_snapshot(
    app: &AppHandle,
    session: Option<&GlobalSystemMediaTransportControlsSession>,
    last_track_id: &mut String,
    last_artwork: &mut Option<String>,
    last_state: &mut MediaState,
    art_attempts: &mut u8,
) {
    if let Some(session) = session {
        if let Some((state, track_id, artwork)) =
            poll_session(session, last_track_id, last_artwork, art_attempts)
        {
            let art_changed = track_id != *last_track_id || artwork != *last_artwork;
            *last_track_id = track_id;
            *last_artwork = artwork;
            *last_state = state.clone();
            if let Ok(mut cached) = SNAPSHOT.lock() {
                *cached = Some(state.clone());
            }
            let mut event = state;
            if !art_changed {
                event.artwork = None;
            }
            let _ = app.emit("media", event);
            return;
        }
    }

    if last_state.active {
        *last_state = MediaState::default();
        last_track_id.clear();
        *last_artwork = None;
        if let Ok(mut cached) = SNAPSHOT.lock() {
            *cached = Some(last_state.clone());
        }
        let _ = app.emit("media", last_state.clone());
    }
}

/// Safety refresh interval while a GSMTC manager is bound.
pub const MEDIA_SAFETY_REFRESH: std::time::Duration = std::time::Duration::from_secs(30);

/// Delay before retrying a failed GSMTC manager request: 2s, 5s, 10s, then every 30s.
pub fn manager_retry_delay(failures: u32) -> std::time::Duration {
    const STEPS: [u64; 3] = [2, 5, 10];
    let index = failures.saturating_sub(1) as usize;
    STEPS
        .get(index)
        .map(|seconds| std::time::Duration::from_secs(*seconds))
        .unwrap_or(MEDIA_SAFETY_REFRESH)
}

/// Starts an event-driven GSMTC service. A 30-second safety refresh remains because
/// some players miss artwork/timeline notifications during process startup.
pub fn start(app: AppHandle) {
    #[cfg(windows)]
    {
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut guard) = SERVICE_TX.lock() {
            *guard = Some(sender.clone());
        }
        if let Err(error) = std::thread::Builder::new()
            .name("tuck-media-events".into())
            .spawn(move || run_event_service(app, sender, receiver))
        {
            eprintln!("Could not start media service: {error}");
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

#[cfg(windows)]
fn run_event_service(
    app: AppHandle,
    sender: mpsc::Sender<MediaSignal>,
    receiver: mpsc::Receiver<MediaSignal>,
) {
    unsafe {
        if let Err(error) = CoInitializeEx(None, COINIT_MULTITHREADED).ok() {
            eprintln!("Media COM startup: {error}");
            return;
        }
    }

    let mut manager: Option<GlobalSystemMediaTransportControlsSessionManager> = None;
    let mut manager_subscriptions: Option<ManagerSubscriptions> = None;
    let mut session_subscriptions: Option<SessionSubscriptions> = None;
    let mut current: Option<GlobalSystemMediaTransportControlsSession> = None;
    let mut last_track_id = String::new();
    let mut last_artwork: Option<String> = None;
    let mut last_state = MediaState::default();
    let mut art_attempts = 0;
    let mut suspended = false;
    let mut rebuild_all = true;
    let mut rebind_session = true;
    let mut manager_failures = 0u32;

    loop {
        if !suspended && rebuild_all {
            drop(session_subscriptions.take());
            drop(manager_subscriptions.take());
            current = None;
            manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
                .ok()
                .and_then(|operation| operation.get().ok());
            if let Some(value) = manager.as_ref() {
                manager_failures = 0;
                manager_subscriptions = ManagerSubscriptions::new(value, &sender)
                    .map_err(|error| eprintln!("Media manager subscription: {error}"))
                    .ok();
                // Without subscriptions new players are never seen; retry like a failed manager
                if manager_subscriptions.is_none() {
                    manager = None;
                    manager_failures = manager_failures.saturating_add(1);
                }
            } else {
                manager_failures = manager_failures.saturating_add(1);
                eprintln!("Media manager unavailable (attempt {manager_failures})");
            }
            rebuild_all = false;
            rebind_session = true;
        }

        if !suspended && rebind_session {
            drop(session_subscriptions.take());
            current = manager.as_ref().and_then(session_from_manager);
            if let Some(value) = current.as_ref() {
                session_subscriptions = SessionSubscriptions::new(value, &sender)
                    .map_err(|error| eprintln!("Media session subscription: {error}"))
                    .ok();
            }
            emit_snapshot(
                &app,
                current.as_ref(),
                &mut last_track_id,
                &mut last_artwork,
                &mut last_state,
                &mut art_attempts,
            );
            rebind_session = false;
        }

        let waiting_for_manager = !suspended && manager.is_none();
        let timeout = if waiting_for_manager {
            manager_retry_delay(manager_failures)
        } else {
            MEDIA_SAFETY_REFRESH
        };
        let signal = receiver.recv_timeout(timeout);
        match signal {
            Ok(MediaSignal::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(MediaSignal::Suspend) => {
                suspended = true;
                drop(session_subscriptions.take());
                drop(manager_subscriptions.take());
                current = None;
                manager = None;
            }
            Ok(MediaSignal::RebindAll) => {
                suspended = false;
                rebuild_all = true;
                manager_failures = 0;
            }
            Err(mpsc::RecvTimeoutError::Timeout) if waiting_for_manager => rebuild_all = true,
            Ok(MediaSignal::ManagerChanged) => rebind_session = true,
            Ok(MediaSignal::SessionChanged) | Err(mpsc::RecvTimeoutError::Timeout) => {
                if !suspended {
                    emit_snapshot(
                        &app,
                        current.as_ref(),
                        &mut last_track_id,
                        &mut last_artwork,
                        &mut last_state,
                        &mut art_attempts,
                    );
                }
            }
        }
    }

    drop((
        session_subscriptions,
        manager_subscriptions,
        current,
        manager,
    ));
    if let Ok(mut guard) = SERVICE_TX.lock() {
        *guard = None;
    }
    unsafe { CoUninitialize() };
}

pub fn suspend() {
    #[cfg(windows)]
    request(MediaSignal::Suspend);
}

pub fn resume() {
    #[cfg(windows)]
    request(MediaSignal::RebindAll);
}

pub fn stop() {
    #[cfg(windows)]
    request(MediaSignal::Stop);
}

#[cfg(windows)]
struct Apartment;
#[cfg(windows)]
impl Apartment {
    fn enter() -> Result<Self, String> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }
            .map_err(|_| "Media service unavailable".to_string())?;
        Ok(Self)
    }
}
#[cfg(windows)]
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

async fn control(action: u8, ms: i64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(windows)]
        {
            let _apartment = Apartment::enter()?;
            let session = current_session().ok_or("No active media session")?;
            let result = match action {
                0 => {
                    let info = session
                        .GetPlaybackInfo()
                        .map_err(|_| "Playback state unavailable")?;
                    let controls = info
                        .Controls()
                        .map_err(|_| "Playback controls unavailable")?;
                    if controls.IsPlayPauseToggleEnabled().unwrap_or(false) {
                        session.TryTogglePlayPauseAsync().and_then(|op| op.get())
                    } else if info.PlaybackStatus().ok()
                        == Some(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
                    {
                        session.TryPauseAsync().and_then(|op| op.get())
                    } else {
                        session.TryPlayAsync().and_then(|op| op.get())
                    }
                }
                1 => session.TrySkipNextAsync().and_then(|op| op.get()),
                2 => session.TrySkipPreviousAsync().and_then(|op| op.get()),
                _ => {
                    let timeline = session
                        .GetTimelineProperties()
                        .map_err(|_| "Timeline unavailable")?;
                    let start = timeline
                        .StartTime()
                        .map_err(|_| "Timeline unavailable")?
                        .Duration;
                    let end = timeline
                        .EndTime()
                        .map_err(|_| "Timeline unavailable")?
                        .Duration;
                    let duration_ms = ticks_to_ms(end.saturating_sub(start));
                    if duration_ms <= 0 {
                        return Err("This stream does not support seeking".into());
                    }
                    let ticks =
                        start.saturating_add(ms_to_ticks(clamp_seek_ms(ms, Some(duration_ms))));
                    session
                        .TryChangePlaybackPositionAsync(ticks)
                        .and_then(|op| op.get())
                }
            }
            .map_err(|_| "Media action failed")?;
            if result {
                Ok(())
            } else {
                Err("The player did not accept this action".into())
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (action, ms);
            Err("Windows media controls unavailable".into())
        }
    })
    .await
    .map_err(|_| "Media worker failed".to_string())?
}

#[tauri::command]
pub async fn media_toggle() -> Result<(), String> {
    control(0, 0).await
}

#[tauri::command]
pub async fn media_next() -> Result<(), String> {
    control(1, 0).await
}

#[tauri::command]
pub async fn media_prev() -> Result<(), String> {
    control(2, 0).await
}

#[tauri::command]
pub async fn media_seek(ms: i64) -> Result<(), String> {
    control(3, ms).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_retry_backs_off_to_safety_refresh() {
        use std::time::Duration;
        assert_eq!(manager_retry_delay(0), Duration::from_secs(2));
        assert_eq!(manager_retry_delay(1), Duration::from_secs(2));
        assert_eq!(manager_retry_delay(2), Duration::from_secs(5));
        assert_eq!(manager_retry_delay(3), Duration::from_secs(10));
        assert_eq!(manager_retry_delay(4), MEDIA_SAFETY_REFRESH);
        assert_eq!(manager_retry_delay(u32::MAX), MEDIA_SAFETY_REFRESH);
        assert_eq!(MEDIA_SAFETY_REFRESH, Duration::from_secs(30));
    }

    #[test]
    fn test_clamp_seek_ms() {
        assert_eq!(clamp_seek_ms(-100, Some(5000)), 0);
        assert_eq!(clamp_seek_ms(2500, Some(5000)), 2500);
        assert_eq!(clamp_seek_ms(6000, Some(5000)), 5000);
        assert_eq!(clamp_seek_ms(1200, None), 1200);
        assert_eq!(clamp_seek_ms(-50, None), 0);
    }

    #[test]
    fn test_ticks_conversion() {
        let ms = 3500;
        let ticks = ms_to_ticks(ms);
        assert_eq!(ticks, 35_000_000);
        assert_eq!(ticks_to_ms(ticks), ms);
    }

    #[test]
    fn test_media_state_serialization() {
        let state = MediaState {
            active: true,
            track_id: "spotify:artist:title:180000".into(),
            title: "Test Title".into(),
            artist: "Test Artist".into(),
            playing: true,
            position_ms: 12000,
            duration_ms: 180000,
            artwork: Some("data:image/jpeg;base64,abc".into()),
            session_id: Some("Spotify.exe:0".into()),
            source_app_id: Some("Spotify.exe".into()),
            can_play_pause: true,
            can_skip_next: true,
            can_skip_prev: false,
            can_seek: true,
            generation: 42,
        };

        let json = serde_json::to_string(&state).expect("serialize");
        assert!(json.contains("\"trackId\":"));
        assert!(json.contains("\"sessionId\":"));
        assert!(json.contains("\"sourceAppId\":"));
        assert!(json.contains("\"canPlayPause\":true"));
        assert!(json.contains("\"canSkipPrev\":false"));
        assert!(json.contains("\"canSeek\":true"));
        assert!(json.contains("\"generation\":42"));

        let deserialized: MediaState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, state);
    }

    #[test]
    fn test_media_session_selection() {
        assert!(select_session_id(Some("Spotify.exe:1".into())).is_ok());
        {
            let sel = SELECTED_SESSION_ID.lock().unwrap();
            assert_eq!(sel.as_deref(), Some("Spotify.exe:1"));
        }

        assert!(select_session_id(None).is_ok());
        {
            let sel = SELECTED_SESSION_ID.lock().unwrap();
            assert_eq!(*sel, None);
        }
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "Read-only live Windows media diagnostic; no transport commands"]
    fn live_windows_media() {
        let _apartment = Apartment::enter().expect("COM apartment");
        if let Some(session) = current_session() {
            let mut attempts = 0;
            let (state, _, artwork) =
                poll_session(&session, "", &None, &mut attempts).expect("Media properties");
            println!(
                "LIVE_MEDIA {}",
                serde_json::json!({"active":state.active,"title":state.title,"artist":state.artist,"source":state.source_app_id,"playing":state.playing,"positionMs":state.position_ms,"durationMs":state.duration_ms,"hasArtwork":artwork.is_some(),"canPlayPause":state.can_play_pause,"canSeek":state.can_seek})
            );
        } else {
            println!("LIVE_MEDIA no active Windows session");
        }
    }
}
