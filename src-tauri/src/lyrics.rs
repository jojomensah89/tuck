use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

const MAX_CACHE_ENTRIES: usize = 128;
const DEFAULT_USER_AGENT: &str = "Tuck/0.1.0 (https://github.com/jojomensah89/tuck)";
const LRCLIB_ENDPOINT: &str = "https://lrclib.net/api/get";

static CONFIG: Mutex<LyricsConfig> = Mutex::new(LyricsConfig { enabled: false });
static CACHE: Mutex<Option<VecDeque<(String, LyricsResponse)>>> = Mutex::new(None);
static RATE_LIMIT_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);
static LATEST_TRACK_REQUEST: Mutex<Option<String>> = Mutex::new(None);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LyricsConfig {
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub time_ms: i64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LyricsResponse {
    pub status: String, // "synced", "plain", "instrumental", "not_found", "disabled", "error"
    pub track_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub instrumental: bool,
    pub plain_lyrics: Option<String>,
    pub synced_lyrics: Option<Vec<LyricLine>>,
    pub error_message: Option<String>,
    pub request_seq: u64,
}

#[derive(Deserialize)]
struct LrclibResponse {
    #[serde(default)]
    instrumental: bool,
    #[serde(default, rename = "plainLyrics")]
    plain_lyrics: Option<String>,
    #[serde(default, rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
}

/// Parses LRC timestamp tag [mm:ss.xx] or [mm:ss.xxx] into milliseconds.
pub fn parse_lrc_timestamp(tag: &str) -> Option<i64> {
    let trimmed = tag.trim().trim_start_matches('[').trim_end_matches(']');
    let parts: Vec<&str> = trimmed.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let minutes: i64 = parts[0].parse().ok()?;
    let sec_parts: Vec<&str> = parts[1].split('.').collect();
    if sec_parts.is_empty() || sec_parts.len() > 2 {
        return None;
    }
    let seconds: i64 = sec_parts[0].parse().ok()?;
    let ms_fraction: i64 = if sec_parts.len() == 2 {
        let frac_str = sec_parts[1];
        if frac_str.is_empty() {
            0
        } else {
            let parsed: i64 = frac_str.parse().ok()?;
            match frac_str.len() {
                1 => parsed * 100,
                2 => parsed * 10,
                3 => parsed,
                _ => {
                    let clamped = &frac_str[..3];
                    clamped.parse().unwrap_or(0)
                }
            }
        }
    } else {
        0
    };

    Some(minutes * 60_000 + seconds * 1_000 + ms_fraction)
}

/// Parses standard LRC formatted text into structured, sorted `LyricLine` vector.
pub fn parse_lrc(lrc_content: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();

    for raw_line in lrc_content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Skip metadata tags like [ar:Artist], [ti:Title], [length:03:45], [al:Album], etc.
        if trimmed.starts_with("[ar:")
            || trimmed.starts_with("[ti:")
            || trimmed.starts_with("[al:")
            || trimmed.starts_with("[by:")
            || trimmed.starts_with("[length:")
            || trimmed.starts_with("[offset:")
            || trimmed.starts_with("[re:")
            || trimmed.starts_with("[ve:")
        {
            continue;
        }

        // Collect all timestamps at start of line
        let mut timestamps = Vec::new();
        let mut cursor = trimmed;

        while cursor.starts_with('[') {
            if let Some(close_bracket) = cursor.find(']') {
                let tag = &cursor[..=close_bracket];
                if let Some(time_ms) = parse_lrc_timestamp(tag) {
                    timestamps.push(time_ms);
                    cursor = cursor[close_bracket + 1..].trim_start();
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        let lyric_text = cursor.trim().to_string();
        for ts in timestamps {
            lines.push(LyricLine {
                time_ms: ts,
                text: lyric_text.clone(),
            });
        }
    }

    lines.sort_by_key(|l| l.time_ms);
    lines
}

fn cache_key(artist: &str, title: &str, duration_ms: i64) -> String {
    let dur_sec = if duration_ms > 0 {
        duration_ms / 1000
    } else {
        0
    };
    format!(
        "{}:{}:{}",
        artist.trim().to_lowercase(),
        title.trim().to_lowercase(),
        dur_sec
    )
}

fn check_authorized_window(window: Option<&tauri::Window>) -> Result<(), String> {
    if let Some(w) = window {
        let label = w.label();
        if label != "main" && label != "settings" {
            return Err(
                "Access denied: only main and settings windows may perform this action".into(),
            );
        }
    }
    Ok(())
}

/// Returns current lyrics configuration.
#[tauri::command]
pub fn lyrics_status() -> LyricsConfig {
    CONFIG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Enables or disables lyrics lookup. Restricted to main/settings window.
#[tauri::command]
pub fn lyrics_set_enabled(window: tauri::Window, enabled: bool) -> Result<(), String> {
    check_authorized_window(Some(&window))?;
    let _preference = crate::service_preferences::write(window.app_handle(), "lyrics", enabled)?;
    let mut config = CONFIG.lock().map_err(|_| "Failed to lock lyrics config")?;
    config.enabled = enabled;
    drop(config);
    // Settings and island are separate windows.
    window
        .app_handle()
        .emit("lyrics-settings", LyricsConfig { enabled })
        .map_err(|_| "Could not notify lyrics setting")?;
    Ok(())
}

pub fn start(app: &tauri::AppHandle) {
    let enabled = crate::service_preferences::read(app, "lyrics");
    if let Ok(mut config) = CONFIG.lock() {
        config.enabled = enabled;
    }
}

/// Clears cached lyrics. Restricted to main/settings window.
#[tauri::command]
pub fn lyrics_clear_cache(window: tauri::Window) -> Result<(), String> {
    check_authorized_window(Some(&window))?;
    let mut cache_guard = CACHE.lock().map_err(|_| "Failed to lock lyrics cache")?;
    *cache_guard = None;
    Ok(())
}

/// Fetches lyrics from LRCLIB for the given track.
/// Respects opt-in configuration, caching, rate-limit backoff, and stale request detection.
#[tauri::command]
pub async fn lyrics_get(
    title: String,
    artist: String,
    duration_ms: i64,
    track_id: Option<String>,
) -> Result<LyricsResponse, String> {
    if title.trim().is_empty()
        || title.len() > 2048
        || artist.len() > 2048
        || duration_ms > 3_600_000
    {
        return Err("Track metadata is unavailable or outside lyrics lookup limits".into());
    }
    let seq = REQUEST_SEQ.fetch_add(1, Ordering::SeqCst);

    if let Some(ref tid) = track_id {
        if let Ok(mut last) = LATEST_TRACK_REQUEST.lock() {
            *last = Some(tid.clone());
        }
    }

    // 1. Check if opt-in enabled
    let is_enabled = {
        let cfg = CONFIG.lock().map_err(|_| "Failed to lock lyrics config")?;
        cfg.enabled
    };

    if !is_enabled {
        return Ok(LyricsResponse {
            status: "disabled".into(),
            track_id,
            title,
            artist,
            instrumental: false,
            plain_lyrics: None,
            synced_lyrics: None,
            error_message: Some("Lyrics lookup is disabled in settings".into()),
            request_seq: seq,
        });
    }

    let key = cache_key(&artist, &title, duration_ms);

    // 2. Check cache
    {
        if let Ok(cache_guard) = CACHE.lock() {
            if let Some(ref deque) = *cache_guard {
                if let Some((_, cached)) = deque.iter().find(|(k, _)| k == &key) {
                    let mut resp = cached.clone();
                    resp.track_id = track_id;
                    resp.request_seq = seq;
                    return Ok(resp);
                }
            }
        }
    }

    // 3. Check rate-limit cooldown
    {
        if let Ok(rl_guard) = RATE_LIMIT_UNTIL.lock() {
            if let Some(until) = *rl_guard {
                if Instant::now() < until {
                    return Ok(LyricsResponse {
                        status: "error".into(),
                        track_id,
                        title,
                        artist,
                        instrumental: false,
                        plain_lyrics: None,
                        synced_lyrics: None,
                        error_message: Some("Rate limited: backing off before next request".into()),
                        request_seq: seq,
                    });
                }
            }
        }
    }

    // 4. Fetch asynchronously via reqwest blocking client in spawn_blocking
    let title_clone = title.clone();
    let artist_clone = artist.clone();
    let track_id_clone = track_id.clone();

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<LyricsResponse, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .user_agent(DEFAULT_USER_AGENT)
            .build()
            .map_err(|e| format!("HTTP client build failed: {e}"))?;

        let mut req = client.get(LRCLIB_ENDPOINT);
        req = req.query(&[("track_name", &title_clone), ("artist_name", &artist_clone)]);
        if duration_ms > 0 {
            let dur_sec = duration_ms / 1000;
            req = req.query(&[("duration", &dur_sec.to_string())]);
        }

        // A queued worker must recheck opt-in immediately before network dispatch.
        if !CONFIG
            .lock()
            .map_err(|_| "Lyrics settings unavailable")?
            .enabled
        {
            return Err("Lyrics lookup was disabled".into());
        }
        let response = match req.send() {
            Ok(resp) => resp,
            Err(e) => {
                return Ok(LyricsResponse {
                    status: "error".into(),
                    track_id: track_id_clone,
                    title: title_clone,
                    artist: artist_clone,
                    instrumental: false,
                    plain_lyrics: None,
                    synced_lyrics: None,
                    error_message: Some(format!("Request failed: {e}")),
                    request_seq: seq,
                });
            }
        };

        let status_code = response.status();
        if status_code == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after_secs = response
                .headers()
                .get("Retry-After")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5);

            if let Ok(mut rl) = RATE_LIMIT_UNTIL.lock() {
                *rl = Some(Instant::now() + Duration::from_secs(retry_after_secs.clamp(2, 3600)));
            }

            return Ok(LyricsResponse {
                status: "error".into(),
                track_id: track_id_clone,
                title: title_clone,
                artist: artist_clone,
                instrumental: false,
                plain_lyrics: None,
                synced_lyrics: None,
                error_message: Some("LRCLIB rate limit reached (HTTP 429)".into()),
                request_seq: seq,
            });
        }

        if status_code == reqwest::StatusCode::NOT_FOUND {
            return Ok(LyricsResponse {
                status: "not_found".into(),
                track_id: track_id_clone,
                title: title_clone,
                artist: artist_clone,
                instrumental: false,
                plain_lyrics: None,
                synced_lyrics: None,
                error_message: None,
                request_seq: seq,
            });
        }

        if !status_code.is_success() {
            return Ok(LyricsResponse {
                status: "error".into(),
                track_id: track_id_clone,
                title: title_clone,
                artist: artist_clone,
                instrumental: false,
                plain_lyrics: None,
                synced_lyrics: None,
                error_message: Some(format!("HTTP error {}", status_code.as_u16())),
                request_seq: seq,
            });
        }

        use std::io::Read;
        let mut bytes = Vec::new();
        response
            .take(512 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read lyrics response")?;
        if bytes.len() > 512 * 1024 {
            return Err("Lyrics response exceeded its size limit".into());
        }
        let data: LrclibResponse = match serde_json::from_slice(&bytes) {
            Ok(d) => d,
            Err(e) => {
                return Ok(LyricsResponse {
                    status: "error".into(),
                    track_id: track_id_clone,
                    title: title_clone,
                    artist: artist_clone,
                    instrumental: false,
                    plain_lyrics: None,
                    synced_lyrics: None,
                    error_message: Some(format!("Failed to parse response JSON: {e}")),
                    request_seq: seq,
                });
            }
        };

        if data.instrumental {
            return Ok(LyricsResponse {
                status: "instrumental".into(),
                track_id: track_id_clone,
                title: title_clone,
                artist: artist_clone,
                instrumental: true,
                plain_lyrics: None,
                synced_lyrics: None,
                error_message: None,
                request_seq: seq,
            });
        }

        let (status, synced) = if let Some(ref lrc) = data.synced_lyrics {
            let parsed = parse_lrc(lrc);
            if !parsed.is_empty() {
                ("synced".to_string(), Some(parsed))
            } else if data.plain_lyrics.is_some() {
                ("plain".to_string(), None)
            } else {
                ("not_found".to_string(), None)
            }
        } else if data.plain_lyrics.is_some() {
            ("plain".to_string(), None)
        } else {
            ("not_found".to_string(), None)
        };

        Ok(LyricsResponse {
            status,
            track_id: track_id_clone,
            title: title_clone,
            artist: artist_clone,
            instrumental: false,
            plain_lyrics: data.plain_lyrics,
            synced_lyrics: synced,
            error_message: None,
            request_seq: seq,
        })
    })
    .await
    .map_err(|e| format!("Lyrics worker thread error: {e}"))??;

    // 5. Store successful response in bounded cache
    if result.status == "synced"
        || result.status == "plain"
        || result.status == "instrumental"
        || result.status == "not_found"
    {
        if let Ok(mut cache_guard) = CACHE.lock() {
            let deque = cache_guard.get_or_insert_with(VecDeque::new);
            if deque.len() >= MAX_CACHE_ENTRIES {
                deque.pop_front();
            }
            deque.push_back((key, result.clone()));
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lrc_timestamp() {
        assert_eq!(parse_lrc_timestamp("[00:12.34]"), Some(12_340));
        assert_eq!(parse_lrc_timestamp("[01:05.123]"), Some(65_123));
        assert_eq!(parse_lrc_timestamp("[02:00.5]"), Some(120_500));
        assert_eq!(parse_lrc_timestamp("[00:00.00]"), Some(0));
        assert_eq!(parse_lrc_timestamp("[invalid]"), None);
        assert_eq!(parse_lrc_timestamp("[12:34:56]"), None);
    }

    #[test]
    fn test_parse_lrc_complete() {
        let sample = "\
[ar:Coldplay]
[ti:Yellow]
[00:10.50]Look at the stars
[00:15.20][00:25.00]Look how they shine for you
[00:20.00]And everything you do
";
        let lines = parse_lrc(sample);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].time_ms, 10_500);
        assert_eq!(lines[0].text, "Look at the stars");

        assert_eq!(lines[1].time_ms, 15_200);
        assert_eq!(lines[1].text, "Look how they shine for you");

        assert_eq!(lines[2].time_ms, 20_000);
        assert_eq!(lines[2].text, "And everything you do");

        assert_eq!(lines[3].time_ms, 25_000);
        assert_eq!(lines[3].text, "Look how they shine for you");
    }

    #[test]
    fn test_lyrics_opt_in_default() {
        assert!(!lyrics_status().enabled);
    }

    #[test]
    fn test_cache_key_normalization() {
        let k1 = cache_key(" Coldplay ", "Yellow", 269000);
        let k2 = cache_key("coldplay", "yellow", 269500);
        assert_eq!(k1, k2);
    }
}
