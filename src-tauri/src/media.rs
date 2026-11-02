use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};
static SNAPSHOT: Mutex<Option<MediaState>> = Mutex::new(None);

#[tauri::command]
pub fn media_now() -> MediaState {
    SNAPSHOT
        .lock()
        .ok()
        .and_then(|state| state.clone())
        .unwrap_or_default()
}

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

/// Media state contract matching frontend and guide.
#[derive(Serialize, Clone, Default, Debug, PartialEq)]
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
fn current_session() -> Option<GlobalSystemMediaTransportControlsSession> {
    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .ok()?
        .get()
        .ok()?;
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
    if track_id != last_track_id {
        *art_attempts = 0;
    }

    // Reuse cached artwork if track identity hasn't changed to avoid reading stream on every poll
    let artwork = if track_id == last_track_id && last_artwork.is_some() {
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
    };

    Some((state, track_id, artwork))
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
pub(crate) async fn media_toggle() -> Result<(), String> {
    control(0, 0).await
}
#[tauri::command]
pub(crate) async fn media_next() -> Result<(), String> {
    control(1, 0).await
}
#[tauri::command]
pub(crate) async fn media_prev() -> Result<(), String> {
    control(2, 0).await
}
#[tauri::command]
pub(crate) async fn media_seek(ms: i64) -> Result<(), String> {
    control(3, ms).await
}
