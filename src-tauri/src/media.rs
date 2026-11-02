use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

static SNAPSHOT: Mutex<Option<MediaState>> = Mutex::new(None);
static SELECTED_SESSION_ID: Mutex<Option<String>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(1);

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
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
#[cfg(windows)]
use windows::Storage::Streams::DataReader;
#[cfg(windows)]
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

#[cfg(windows)]
fn current_session() -> Option<GlobalSystemMediaTransportControlsSession> {
    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .ok()?
        .get()
        .ok()?;

    let selected = SELECTED_SESSION_ID.lock().ok()?.clone();
    if let Some(target_id) = selected {
        if !target_id.trim().is_empty() {
            if let Ok(sessions) = mgr.GetSessions() {
                if let Ok(count) = sessions.Size() {
                    for i in 0..count {
                        if let Ok(s) = sessions.GetAt(i) {
                            let aumid = s
                                .SourceAppUserModelId()
                                .map(|id| id.to_string())
                                .unwrap_or_default();
                            let formatted_id = format!("{aumid}:{i}");
                            if aumid == target_id || formatted_id == target_id {
                                return Some(s);
                            }
                        }
                    }
                }
            }
        }
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
            for i in 0..count {
                if let Ok(s) = sessions.GetAt(i) {
                    let source_app_id = s
                        .SourceAppUserModelId()
                        .map(|id| id.to_string())
                        .unwrap_or_default();
                    let session_id = format!("{source_app_id}:{i}");

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
                        .map(|sel| sel == session_id || sel == source_app_id)
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
    window: Option<tauri::Window>,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref w) = window {
        let label = w.label();
        if label != "main" && label != "settings" {
            return Err(
                "Access denied: only main and settings windows may select media sessions".into(),
            );
        }
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
    Ok(())
}

/// Starts the 500ms background SMTC poller.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let mut last_track_id = String::new();
        let mut last_artwork: Option<String> = None;
        let mut last_state = MediaState::default();
        let mut art_attempts = 0;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));

            #[cfg(windows)]
            {
                if let Some(session) = current_session() {
                    if let Some((state, track_id, artwork)) =
                        poll_session(&session, &last_track_id, &last_artwork, &mut art_attempts)
                    {
                        let art_changed = track_id != last_track_id || artwork != last_artwork;
                        last_track_id = track_id;
                        last_artwork = artwork;
                        last_state = state.clone();
                        if let Ok(mut cached) = SNAPSHOT.lock() {
                            *cached = Some(state.clone());
                        }
                        let mut event = state;
                        if !art_changed {
                            event.artwork = None;
                        }
                        let _ = app.emit("media", event);
                        continue;
                    }
                }
            }

            // No active session or player exited
            if last_state.active {
                last_state = MediaState::default();
                last_track_id.clear();
                last_artwork = None;
                if let Ok(mut cached) = SNAPSHOT.lock() {
                    *cached = Some(last_state.clone());
                }
                let _ = app.emit("media", last_state.clone());
            }
        }
    });
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
                0 => session.TryTogglePlayPauseAsync().and_then(|op| op.get()),
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
        assert!(media_select_session(None, Some("Spotify.exe:1".into())).is_ok());
        {
            let sel = SELECTED_SESSION_ID.lock().unwrap();
            assert_eq!(sel.as_deref(), Some("Spotify.exe:1"));
        }

        assert!(media_select_session(None, None).is_ok());
        {
            let sel = SELECTED_SESSION_ID.lock().unwrap();
            assert_eq!(*sel, None);
        }
    }
}
