use crate::lyrics_match::{resolve_match, Candidate, LrclibRecord, Track};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

const MAX_CACHE_ENTRIES: usize = 128;
const NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const DEFAULT_USER_AGENT: &str = "Tuck/0.1.0 (https://github.com/jojomensah89/tuck)";
const LRCLIB_GET_ENDPOINT: &str = "https://lrclib.net/api/get";
const LRCLIB_SEARCH_ENDPOINT: &str = "https://lrclib.net/api/search";
/// LRCLIB asks for 200-500 ms between sequential requests.
const REQUEST_GAP: Duration = Duration::from_millis(250);
const MAX_RECORD_BYTES: u64 = 512 * 1024;
const MAX_SEARCH_BYTES: u64 = 2 * 1024 * 1024;

static CONFIG: Mutex<LyricsConfig> = Mutex::new(LyricsConfig { enabled: false });
static CACHE: Mutex<Option<VecDeque<CacheEntry>>> = Mutex::new(None);
static RATE_LIMIT_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);
static LATEST_TRACK_REQUEST: Mutex<Option<String>> = Mutex::new(None);
static HTTP_CLIENT: OnceLock<Result<reqwest::blocking::Client, String>> = OnceLock::new();

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
    /// The song matched, but its timing or credits differ from what is playing.
    pub approximate: bool,
}

#[derive(Clone)]
struct CacheEntry {
    key: String,
    response: LyricsResponse,
    stored_at: Instant,
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
/// A positive `[offset:]` makes lyrics appear sooner, per the format's definition.
pub fn parse_lrc(lrc_content: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    let offset_ms = lrc_content
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("[offset:")?
                .strip_suffix(']')?
                .trim()
                .parse::<i64>()
                .ok()
        })
        .unwrap_or(0);

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
                time_ms: (ts - offset_ms).max(0),
                text: lyric_text.clone(),
            });
        }
    }

    lines.sort_by_key(|l| l.time_ms);
    lines
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

fn http_client() -> Result<&'static reqwest::blocking::Client, String> {
    HTTP_CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .user_agent(DEFAULT_USER_AGENT)
                .build()
                .map_err(|error| format!("HTTP client build failed: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn request_is_current(track_id: Option<&str>) -> bool {
    let Some(track_id) = track_id else {
        return true;
    };
    LATEST_TRACK_REQUEST
        .lock()
        .map(|latest| latest.as_deref() == Some(track_id))
        .unwrap_or(false)
}

/// Must pass immediately before every network request: the user can opt out, change track or
/// hit the rate limit while an earlier request of the same lookup is in flight.
fn preflight_request(track_id: Option<&str>) -> Result<(), String> {
    if !CONFIG
        .lock()
        .map_err(|_| "Lyrics settings unavailable")?
        .enabled
    {
        return Err("Lyrics lookup was disabled".into());
    }
    if !request_is_current(track_id) {
        return Err("Lyrics request was superseded by another track".into());
    }
    if RATE_LIMIT_UNTIL
        .lock()
        .ok()
        .and_then(|value| *value)
        .is_some_and(|until| Instant::now() < until)
    {
        return Err("Rate limited: backing off before next request".into());
    }
    Ok(())
}

/// LRCLIB asks clients to honour `Retry-After` and add up to about 30% jitter.
fn rate_limit_backoff(retry_after_secs: u64, entropy: u32) -> Duration {
    let base_ms = retry_after_secs.clamp(2, 3600) * 1000;
    Duration::from_millis(base_ms + base_ms * u64::from(entropy % 301) / 1000)
}

fn remember_rate_limit(response: &reqwest::blocking::Response) {
    let retry_after_secs = response
        .headers()
        .get("Retry-After")
        .and_then(|header| header.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(5);
    let entropy = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    if let Ok(mut until) = RATE_LIMIT_UNTIL.lock() {
        *until = Some(Instant::now() + rate_limit_backoff(retry_after_secs, entropy));
    }
}

/// One LRCLIB request. `Ok(None)` is its "not found"; anything else unexpected is an `Err`.
fn send_json<T: serde::de::DeserializeOwned>(
    url: &str,
    query: &[(&str, String)],
    max_bytes: u64,
) -> Result<Option<T>, String> {
    use std::io::Read;
    let response = http_client()?
        .get(url)
        .query(query)
        .send()
        .map_err(|error| format!("Request failed: {error}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        remember_rate_limit(&response);
        return Err("LRCLIB rate limit reached (HTTP 429)".into());
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(format!("HTTP error {}", status.as_u16()));
    }
    let mut bytes = Vec::new();
    response
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read lyrics response")?;
    if bytes.len() as u64 > max_bytes {
        return Err("Lyrics response exceeded its size limit".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("Failed to parse response JSON: {error}"))
}

/// Parameters for `/api/get`, whose `duration` must be at least 1 s when sent.
fn exact_query(title: &str, artist: &str, duration_s: Option<f64>) -> Vec<(&'static str, String)> {
    let mut query = vec![
        ("track_name", title.to_string()),
        ("artist_name", artist.to_string()),
    ];
    if let Some(duration) = duration_s {
        query.push(("duration", (duration.round() as i64).to_string()));
    }
    query
}

fn fetch_exact(
    title: &str,
    artist: &str,
    duration_s: Option<f64>,
) -> Result<Option<LrclibRecord>, String> {
    send_json(
        LRCLIB_GET_ENDPOINT,
        &exact_query(title, artist, duration_s),
        MAX_RECORD_BYTES,
    )
}

/// `/api/search` results, skipping any record that does not parse rather than losing them all.
fn fetch_search(wanted: &Track) -> Result<Option<Vec<LrclibRecord>>, String> {
    let records: Option<Vec<serde_json::Value>> = send_json(
        LRCLIB_SEARCH_ENDPOINT,
        &wanted.search_query(),
        MAX_SEARCH_BYTES,
    )?;
    Ok(records.map(|records| {
        records
            .into_iter()
            .filter_map(|record| serde_json::from_value(record).ok())
            .collect()
    }))
}

fn lookup(
    title: &str,
    artist: &str,
    duration_s: Option<f64>,
    track_id: Option<&str>,
) -> Result<Option<Candidate>, String> {
    let wanted = Track::parse(title, artist);
    resolve_match(
        &wanted,
        duration_s,
        || {
            preflight_request(track_id)?;
            fetch_exact(title, artist, duration_s)
        },
        || {
            std::thread::sleep(REQUEST_GAP);
            preflight_request(track_id)?;
            fetch_search(&wanted)
        },
    )
}

fn response_without_lyrics(
    status: &str,
    track_id: Option<String>,
    title: String,
    artist: String,
    seq: u64,
    error_message: Option<String>,
) -> LyricsResponse {
    LyricsResponse {
        status: status.into(),
        track_id,
        title,
        artist,
        instrumental: false,
        plain_lyrics: None,
        synced_lyrics: None,
        error_message,
        request_seq: seq,
        approximate: false,
    }
}

fn response_from_candidate(
    candidate: Candidate,
    track_id: Option<String>,
    title: String,
    artist: String,
    seq: u64,
) -> LyricsResponse {
    let record = candidate.record;
    let synced = (!record.instrumental && !candidate.lines.is_empty()).then_some(candidate.lines);
    let plain = record
        .plain_lyrics
        .filter(|plain| !record.instrumental && !plain.trim().is_empty());
    let status = if record.instrumental {
        "instrumental"
    } else if synced.is_some() {
        "synced"
    } else {
        "plain"
    };
    LyricsResponse {
        status: status.into(),
        track_id,
        title,
        artist,
        instrumental: record.instrumental,
        plain_lyrics: plain,
        synced_lyrics: synced,
        error_message: None,
        request_seq: seq,
        approximate: candidate.approximate,
    }
}

fn cached_response(key: &str) -> Option<LyricsResponse> {
    let mut guard = CACHE.lock().ok()?;
    let entries = guard.as_mut()?;
    entries.retain(|entry| {
        entry.response.status != "not_found" || entry.stored_at.elapsed() < NEGATIVE_CACHE_TTL
    });
    entries
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| entry.response.clone())
}

/// Remembers a real answer. A miss without a duration is not one: search cannot accept anything
/// without a length to check, and the player may simply not have reported it yet.
fn store_response(key: String, response: &LyricsResponse, duration_s: Option<f64>) {
    if response.status == "not_found" && duration_s.is_none() {
        return;
    }
    if let Ok(mut guard) = CACHE.lock() {
        let entries = guard.get_or_insert_with(VecDeque::new);
        if entries.len() >= MAX_CACHE_ENTRIES {
            entries.pop_front();
        }
        entries.push_back(CacheEntry {
            key,
            response: response.clone(),
            stored_at: Instant::now(),
        });
    }
}

fn can_look_up(title: &str, artist: &str, duration_ms: i64) -> bool {
    !title.trim().is_empty()
        && !artist.trim().is_empty()
        && title.len() <= 2048
        && artist.len() <= 2048
        && duration_ms <= 3_600_000
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
    let seq = REQUEST_SEQ.fetch_add(1, Ordering::SeqCst);

    if let Some(ref tid) = track_id {
        if let Ok(mut last) = LATEST_TRACK_REQUEST.lock() {
            *last = Some(tid.clone());
        }
    }

    let is_enabled = {
        let cfg = CONFIG.lock().map_err(|_| "Failed to lock lyrics config")?;
        cfg.enabled
    };
    if !is_enabled {
        return Ok(response_without_lyrics(
            "disabled",
            track_id,
            title,
            artist,
            seq,
            Some("Lyrics lookup is disabled in settings".into()),
        ));
    }
    if !can_look_up(&title, &artist, duration_ms) {
        // Nothing to search for, which is not a failure and not worth remembering.
        return Ok(response_without_lyrics(
            "not_found",
            track_id,
            title,
            artist,
            seq,
            None,
        ));
    }

    let duration_s = (duration_ms >= 1000).then_some(duration_ms as f64 / 1000.0);
    let key = Track::parse(&title, &artist).cache_key(duration_s);
    if let Some(mut cached) = cached_response(&key) {
        cached.track_id = track_id;
        cached.request_seq = seq;
        return Ok(cached);
    }

    let (lookup_title, lookup_artist, lookup_track) =
        (title.clone(), artist.clone(), track_id.clone());
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        lookup(
            &lookup_title,
            &lookup_artist,
            duration_s,
            lookup_track.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("Lyrics worker thread error: {e}"))?;

    // Failures can clear on their own, so only real answers are remembered.
    Ok(match outcome {
        Ok(found) => {
            let response = match found {
                Some(candidate) => response_from_candidate(candidate, track_id, title, artist, seq),
                None => response_without_lyrics("not_found", track_id, title, artist, seq, None),
            };
            store_response(key, &response, duration_s);
            response
        }
        Err(error) => response_without_lyrics("error", track_id, title, artist, seq, Some(error)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics_match::best_search_match;

    /// The cache is shared and bounded, so tests that fill it must not run alongside those that
    /// read it back.
    static CACHE_TESTS: Mutex<()> = Mutex::new(());

    fn answer(status: &str) -> LyricsResponse {
        response_without_lyrics(status, None, String::new(), String::new(), 1, None)
    }

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
    fn test_parse_lrc_honors_offset_tag() {
        let lines = parse_lrc("[offset:+250]\n[00:01.00]Sooner\n[00:02.00]Later");
        assert_eq!(lines[0].time_ms, 750, "a positive offset means sooner");
        assert_eq!(lines[1].time_ms, 1_750);
        assert_eq!(
            parse_lrc("[offset: -250]\n[00:01.00]Later")[0].time_ms,
            1_250
        );
    }

    #[test]
    fn test_lyrics_opt_in_default() {
        assert!(!lyrics_status().enabled);
        assert!(
            preflight_request(None).is_err(),
            "no network request may start while lyrics are off"
        );
    }

    #[test]
    fn rate_limit_backoff_honours_retry_after_with_bounded_jitter() {
        assert_eq!(rate_limit_backoff(10, 0), Duration::from_secs(10));
        assert_eq!(rate_limit_backoff(10, 300), Duration::from_secs(13));
        assert_eq!(rate_limit_backoff(0, 0), Duration::from_secs(2));
        assert!(rate_limit_backoff(u64::MAX, u32::MAX) <= Duration::from_secs(3600 * 13 / 10));
    }

    #[test]
    fn an_answer_always_has_a_lyrics_status() {
        let record = |synced: Option<&str>, plain: Option<&str>, instrumental| LrclibRecord {
            track_name: "Long Song Title".into(),
            artist_name: "Aster".into(),
            duration: Some(200.0),
            instrumental,
            plain_lyrics: plain.map(String::from),
            synced_lyrics: synced.map(String::from),
        };
        let status = |record| {
            let wanted = Track::parse("Long Song Title", "Aster");
            let candidate = crate::lyrics_match::score_candidate(&wanted, record, Some(200.0))
                .expect("eligible record");
            response_from_candidate(candidate, None, String::new(), String::new(), 1).status
        };
        let lrc = Some("[00:01.00]Line");
        assert_eq!(status(record(lrc, Some("Line"), false)), "synced");
        assert_eq!(status(record(None, Some("Line"), false)), "plain");
        assert_eq!(status(record(None, None, true)), "instrumental");
    }

    #[test]
    fn tracks_without_metadata_are_a_miss_and_not_a_failure() {
        assert!(can_look_up("Song", "Aster", 180_000));
        assert!(
            can_look_up("Song", "Aster", 0),
            "the duration can be unknown"
        );
        assert!(!can_look_up("Song", "  ", 180_000));
        assert!(!can_look_up("  ", "Aster", 180_000));
        assert!(!can_look_up("Song", "Aster", 3_600_001));
    }

    #[test]
    fn a_miss_without_a_duration_does_not_hide_the_track_once_it_is_known() {
        let _guard = CACHE_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let track = Track::parse("Late Duration Song", "Babyboy AV");
        store_response(track.cache_key(None), &answer("not_found"), None);
        assert!(
            cached_response(&track.cache_key(None)).is_none(),
            "the player may just not have reported the duration yet"
        );
        assert!(cached_response(&track.cache_key(Some(171.0))).is_none());

        // A miss that had a duration to check is real, but only for that length.
        store_response(
            track.cache_key(Some(171.0)),
            &answer("not_found"),
            Some(171.0),
        );
        assert!(cached_response(&track.cache_key(Some(171.0))).is_some());
        assert!(cached_response(&track.cache_key(Some(200.0))).is_none());
    }

    #[test]
    fn same_titled_tracks_by_one_artist_do_not_share_an_answer() {
        let _guard = CACHE_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let track = Track::parse("Cached Intro", "Drake");
        store_response(track.cache_key(Some(52.0)), &answer("synced"), Some(52.0));
        assert_eq!(
            cached_response(&track.cache_key(Some(52.0))).map(|found| found.status),
            Some("synced".to_string())
        );
        assert!(cached_response(&track.cache_key(Some(25.0))).is_none());
        assert!(cached_response(&track.cache_key(None)).is_none());
    }

    #[test]
    fn misses_are_cached_until_their_ttl_and_the_cache_stays_bounded() {
        let _guard = CACHE_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let miss = |key: &str| store_response(key.into(), &answer("not_found"), Some(180.0));
        miss("cached miss");
        assert_eq!(
            cached_response("cached miss").map(|response| response.status),
            Some("not_found".to_string()),
            "a failed track must not query LRCLIB again"
        );
        if let Some(entry) = CACHE
            .lock()
            .unwrap()
            .as_mut()
            .and_then(|entries| entries.iter_mut().find(|entry| entry.key == "cached miss"))
        {
            entry.stored_at = Instant::now() - NEGATIVE_CACHE_TTL;
        }
        assert!(
            cached_response("cached miss").is_none(),
            "LRCLIB can gain the track later"
        );

        for index in 0..MAX_CACHE_ENTRIES + 5 {
            miss(&format!("bounded {index}"));
        }
        assert_eq!(
            CACHE.lock().unwrap().as_ref().unwrap().len(),
            MAX_CACHE_ENTRIES
        );
        assert!(
            cached_response("bounded 0").is_none(),
            "oldest entries leave first"
        );
    }

    /// Run with `cargo test live_lrclib -- --ignored --nocapture`.
    #[test]
    #[ignore = "calls the real LRCLIB service"]
    fn live_lrclib_reported_track_uses_search_fallback() {
        let (title, artist, duration) = ("Big Thug Boys", "Babyboy AV", Some(171.0));
        let wanted = Track::parse(title, artist);
        assert!(
            fetch_exact(title, artist, duration)
                .expect("exact request")
                .is_none(),
            "this track is only reachable through search"
        );
        std::thread::sleep(REQUEST_GAP);
        let records = fetch_search(&wanted)
            .expect("search request")
            .expect("search results");
        println!(
            "{:?}",
            records
                .iter()
                .map(|r| (&r.track_name, &r.artist_name, r.duration))
                .collect::<Vec<_>>()
        );
        let best = best_search_match(&wanted, records, duration).expect("accepted match");
        println!(
            "chose {:?} by {} at {:?} s, score {} approximate {} lines {}",
            best.record.track_name,
            best.record.artist_name,
            best.record.duration,
            best.score,
            best.approximate,
            best.lines.len()
        );
        assert!(best.record.track_name.contains("Big Thug Boys"));
        assert_eq!(best.score, 83);
        assert!(best.approximate);
        assert!(!best.lines.is_empty());
    }

    /// One artist, one title, several songs: each length must get its own record.
    #[test]
    #[ignore = "calls the real LRCLIB service"]
    fn live_lrclib_same_title_at_different_durations() {
        let mut found_lengths = Vec::new();
        for duration_s in [Some(52.0), Some(25.0)] {
            let wanted = Track::parse("Intro", "Drake");
            let best = resolve_match(
                &wanted,
                duration_s,
                || fetch_exact("Intro", "Drake", duration_s),
                || {
                    std::thread::sleep(REQUEST_GAP);
                    fetch_search(&wanted)
                },
            )
            .expect("LRCLIB reachable")
            .expect("a record at this length");
            println!(
                "Intro / Drake at {duration_s:?}: {:?} by {} at {:?} s, score {} approximate {} lines {}, key {:?}",
                best.record.track_name,
                best.record.artist_name,
                best.record.duration,
                best.score,
                best.approximate,
                best.lines.len(),
                wanted.cache_key(duration_s),
            );
            let length = best.record.duration.expect("a duration");
            assert!((length - duration_s.unwrap()).abs() <= 3.0);
            found_lengths.push(length);
            std::thread::sleep(REQUEST_GAP);
        }
        assert_ne!(found_lengths[0], found_lengths[1]);
    }

    /// Sanity-checks the query forms against real data, with at most two requests per track.
    #[test]
    #[ignore = "calls the real LRCLIB service"]
    fn live_lrclib_other_tracks() {
        for (title, artist, duration_s, expect_match) in [
            ("With or Without You", "U2", Some(296.0), true),
            ("Back In Black", "AC/DC", Some(255.0), true),
            ("Spring Day", "BTS", Some(274.0), true),
            ("봄날", "방탄소년단", Some(274.0), true),
            ("Big Thug Boys", "Someone Else Entirely", Some(171.0), false),
        ] {
            let wanted = Track::parse(title, artist);
            let found = resolve_match(
                &wanted,
                duration_s,
                || fetch_exact(title, artist, duration_s),
                || {
                    std::thread::sleep(REQUEST_GAP);
                    fetch_search(&wanted)
                },
            )
            .expect("LRCLIB reachable");
            println!(
                "{title} / {artist}: {:?}",
                found.as_ref().map(|found| (
                    &found.record.track_name,
                    &found.record.artist_name,
                    found.record.duration,
                    found.score,
                    found.approximate
                ))
            );
            assert_eq!(found.is_some(), expect_match, "{title} / {artist}");
            std::thread::sleep(REQUEST_GAP);
        }
    }
}
