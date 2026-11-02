use serde::Serialize;
use tauri::{AppHandle, Emitter};

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
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

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

    let track_id = format!("{}:{}:{}", artist, title, duration_ms);

    // Reuse cached artwork if track identity hasn't changed to avoid reading stream on every poll
    let artwork = if track_id == last_track_id {
        last_artwork.clone()
    } else {
        read_artwork(&props)
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

        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));

            #[cfg(windows)]
            {
                if let Some(session) = current_session() {
                    if let Some((state, track_id, artwork)) =
                        poll_session(&session, &last_track_id, &last_artwork)
                    {
                        last_track_id = track_id;
                        last_artwork = artwork;
                        if state != last_state {
                            last_state = state.clone();
                            let _ = app.emit("media", state);
                        }
                        continue;
                    }
                }
            }

            // No active session or player exited
            if last_state.active {
                last_state = MediaState::default();
                last_track_id.clear();
                last_artwork = None;
                let _ = app.emit("media", last_state.clone());
            }
        }
    });
}

#[tauri::command]
pub(crate) async fn media_toggle() {
    tauri::async_runtime::spawn_blocking(|| {
        #[cfg(windows)]
        {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            if let Some(session) = current_session() {
                let _ = session
                    .TryTogglePlayPauseAsync()
                    .ok()
                    .and_then(|o| o.get().ok());
            }
        }
    })
    .await
    .ok();
}

#[tauri::command]
pub(crate) async fn media_next() {
    tauri::async_runtime::spawn_blocking(|| {
        #[cfg(windows)]
        {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            if let Some(session) = current_session() {
                let _ = session.TrySkipNextAsync().ok().and_then(|o| o.get().ok());
            }
        }
    })
    .await
    .ok();
}

#[tauri::command]
pub(crate) async fn media_prev() {
    tauri::async_runtime::spawn_blocking(|| {
        #[cfg(windows)]
        {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            if let Some(session) = current_session() {
                let _ = session
                    .TrySkipPreviousAsync()
                    .ok()
                    .and_then(|o| o.get().ok());
            }
        }
    })
    .await
    .ok();
}

#[tauri::command]
pub(crate) async fn media_seek(ms: i64) {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(windows)]
        {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            if let Some(session) = current_session() {
                let duration_ms = session.GetTimelineProperties().ok().and_then(|tl| {
                    let start = tl.StartTime().map(|d| d.Duration).unwrap_or(0);
                    let end = tl.EndTime().map(|d| d.Duration).unwrap_or(0);
                    let dur = ticks_to_ms(end - start);
                    if dur > 0 {
                        Some(dur)
                    } else {
                        None
                    }
                });
                let clamped_ms = clamp_seek_ms(ms, duration_ms);
                let ticks = ms_to_ticks(clamped_ms);
                let _ = session
                    .TryChangePlaybackPositionAsync(ticks)
                    .ok()
                    .and_then(|o| o.get().ok());
            }
        }
    })
    .await
    .ok();
}
