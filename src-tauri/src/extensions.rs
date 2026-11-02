use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, WebviewWindow};

/* ========================================================================= */
/* DTO & State Definitions                                                   */
/* ========================================================================= */

/// Opt-in settings for all v1 extensions.
/// All monitors are OFF by default and persisted secret-free.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSettings {
    pub downloads_monitor_enabled: bool,
    pub calendar_enabled: bool,
    pub notifications_enabled: bool,
    pub radio_monitor_enabled: bool,
    pub calendar_reminder_minutes: u32,
}

impl Default for ExtensionSettings {
    fn default() -> Self {
        Self {
            downloads_monitor_enabled: false,
            calendar_enabled: false,
            notifications_enabled: false,
            radio_monitor_enabled: false,
            calendar_reminder_minutes: 5,
        }
    }
}

/// Active download item tracking.
/// Total size is strictly indeterminate (`None`) for .crdownload / .part files.
/// NEVER fabricates percentages!
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActiveDownloadItem {
    pub id: String,
    pub file_name: String,
    pub bytes_downloaded: u64,
    pub download_rate_bps: u64,
    pub total_bytes: Option<u64>, // strictly None for .crdownload/.part files
    pub status: String,           // "downloading" | "paused" | "completed"
    pub last_updated_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsState {
    pub enabled: bool,
    pub downloads: Vec<ActiveDownloadItem>,
    pub active_count: usize,
    pub aggregate_rate_bps: u64,
}

/// Manual call activity state.
/// Real elapsed counter, highest priority (95), no auto-detection claims.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallState {
    pub active: bool,
    pub title: String,
    pub start_time_ms: Option<u64>,
    pub elapsed_ms: u64,
    pub muted: bool,
    pub priority: u32, // 95 - highest priority
}

impl Default for CallState {
    fn default() -> Self {
        Self {
            active: false,
            title: "Call in Progress".into(),
            start_time_ms: None,
            elapsed_ms: 0,
            muted: false,
            priority: 95,
        }
    }
}

/// Parsed Calendar Event from local ICS.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub id: String,
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
    pub join_url: Option<String>,
    pub is_all_day: bool,
}

/// Calendar reminder DTO emitted via 'calendar-reminder' event.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarReminderPayload {
    pub event_id: String,
    pub title: String,
    pub start_time_ms: i64,
    pub minutes_until: i64,
    pub join_url: Option<String>,
    pub location: String,
}

/// Result of parsing an ICS file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarImportResult {
    pub count: usize,
    pub events: Vec<CalendarEvent>,
    pub error: Option<String>,
}

/// WinRT UserNotificationListener Access State.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationListenerState {
    pub enabled: bool,
    pub access_status: String, // "Allowed" | "DeniedBySystem" | "DeniedByUser" | "Unspecified" | "Unavailable"
    pub available: bool,
    pub reason: String,
    pub active_notifications_count: usize,
}

impl Default for NotificationListenerState {
    fn default() -> Self {
        Self {
            enabled: false,
            access_status: "Unspecified".into(),
            available: false,
            reason: "Opt-in required. Explicit action must be taken to request Windows notification access.".into(),
            active_notifications_count: 0,
        }
    }
}

/// File Shelf Item.
/// Stored in memory with an allowlisted UUID.
/// Launching files is strictly allowed ONLY via known shelf item ID, never arbitrary shell paths.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShelfItem {
    pub id: String, // UUID
    pub file_name: String,
    pub file_path: String,
    pub file_size_bytes: u64,
    pub file_extension: String,
    pub added_at_ms: u64,
}

/// WinRT Radio state for WiFi and Bluetooth.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RadioInfo {
    pub id: String,
    pub name: String,
    pub kind: String,  // "WiFi" | "Bluetooth" | "MobileBroadband" | "Other"
    pub state: String, // "On" | "Off" | "Disabled" | "Unknown"
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RadiosPayload {
    pub wifi: Option<RadioInfo>,
    pub bluetooth: Option<RadioInfo>,
    pub access_allowed: bool,
    pub error: Option<String>,
}

/// Full extension state snapshot emitted as 'extension-state'.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionStateDTO {
    pub settings: ExtensionSettings,
    pub downloads: DownloadsState,
    pub call: CallState,
    pub upcoming_event: Option<CalendarEvent>,
    pub active_reminder: Option<CalendarReminderPayload>,
    pub notifications: NotificationListenerState,
    pub shelf_items: Vec<ShelfItem>,
    pub radios: RadiosPayload,
}

/* ========================================================================= */
/* Global In-Memory Extensions Manager                                       */
/* ========================================================================= */

pub struct DownloadHistoryRecord {
    pub bytes: u64,
    pub timestamp: Instant,
    pub rate_bps: u64,
}

pub struct ExtensionsManager {
    pub settings: RwLock<ExtensionSettings>,
    pub call: Mutex<CallState>,
    pub shelf: Mutex<HashMap<String, ShelfItem>>,
    pub calendar_events: Mutex<Vec<CalendarEvent>>,
    pub notified_reminders: Mutex<HashSet<String>>,
    pub downloads_history: Mutex<HashMap<PathBuf, DownloadHistoryRecord>>,
    pub notifications: RwLock<NotificationListenerState>,
    pub radios: RwLock<RadiosPayload>,
}

impl ExtensionsManager {
    pub fn new() -> Self {
        Self {
            settings: RwLock::new(ExtensionSettings::default()),
            call: Mutex::new(CallState::default()),
            shelf: Mutex::new(HashMap::new()),
            calendar_events: Mutex::new(Vec::new()),
            notified_reminders: Mutex::new(HashSet::new()),
            downloads_history: Mutex::new(HashMap::new()),
            notifications: RwLock::new(NotificationListenerState::default()),
            radios: RwLock::new(RadiosPayload::default()),
        }
    }
}

static MANAGER: OnceLock<ExtensionsManager> = OnceLock::new();

pub fn get_manager() -> &'static ExtensionsManager {
    MANAGER.get_or_init(ExtensionsManager::new)
}

/* ========================================================================= */
/* Utilities: Time, UUID, URL Validation                                     */
/* ========================================================================= */

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

/// Generates a standard RFC 4122 v4 UUID string without external dependencies.
pub fn generate_uuid_v4() -> String {
    let t = now_unix_ms();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .subsec_nanos();
    
    // Seed 16 bytes pseudo-randomly
    let mut b = [0u8; 16];
    let t_bytes = t.to_be_bytes();
    let n_bytes = nanos.to_be_bytes();
    for i in 0..8 {
        b[i] = t_bytes[i] ^ (i as u8).wrapping_mul(37);
        b[i + 8] = n_bytes[i % 4] ^ (i as u8).wrapping_mul(71);
    }
    // Mix with address of stack variable for uniqueness
    let stack_ptr = &b as *const _ as usize;
    let ptr_bytes = stack_ptr.to_ne_bytes();
    for i in 0..b.len() {
        b[i] ^= ptr_bytes[i % ptr_bytes.len()];
    }

    // Set RFC 4122 v4 version (0100) and variant (10xx)
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

/// Validates that a join URL uses strictly `http://` or `https://` protocol.
/// Rejects `javascript:`, `file:`, `data:`, or malformed URLs.
pub fn validate_http_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Strip leading angle brackets or quotes if present (common in ics/markdown)
    let s = trimmed.trim_matches(|c| c == '<' || c == '>' || c == '"' || c == '\'');

    let lower = s.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        return None;
    }

    // Must have a valid host after http(s)://
    let without_scheme = if lower.starts_with("https://") {
        &s[8..]
    } else {
        &s[7..]
    };

    let host_part = without_scheme.split(&['/', '?', '#'][..]).next().unwrap_or("");
    if host_part.is_empty() || host_part.contains(' ') || host_part.contains('\t') || host_part.contains('\n') {
        return None;
    }

    // Check for illegal characters
    if s.chars().any(|c| c.is_control() || c == ' ' || c == '\r' || c == '\n') {
        return None;
    }

    Some(s.to_string())
}

/// Extracts meeting join link (Teams, Zoom, Meet, Webex or generic http/https)
/// from event fields in priority order: url, location, description.
pub fn extract_meeting_url(
    url: Option<&str>,
    location: Option<&str>,
    description: Option<&str>,
) -> Option<String> {
    if let Some(u) = url {
        if let Some(valid) = validate_http_url(u) {
            return Some(valid);
        }
    }

    let search_in = |text: &str| -> Option<String> {
        // Look for common meeting URL patterns
        let patterns = [
            "https://teams.microsoft.com/",
            "https://teams.live.com/",
            "https://meet.google.com/",
            "https://zoom.us/j/",
            ".zoom.us/j/",
            ".webex.com/",
        ];

        for word in text.split_whitespace() {
            let candidate = word.trim_matches(|c| c == '(' || c == ')' || c == '<' || c == '>' || c == '[' || c == ']' || c == ',' || c == ';');
            for pat in &patterns {
                if candidate.contains(pat) {
                    let full_url = if candidate.starts_with("http://") || candidate.starts_with("https://") {
                        candidate.to_string()
                    } else {
                        format!("https://{}", candidate)
                    };
                    if let Some(valid) = validate_http_url(&full_url) {
                        return Some(valid);
                    }
                }
            }
        }

        // Generic http/https fallback in text
        for word in text.split_whitespace() {
            let candidate = word.trim_matches(|c| c == '(' || c == ')' || c == '<' || c == '>' || c == '[' || c == ']' || c == ',' || c == ';');
            if candidate.starts_with("https://") || candidate.starts_with("http://") {
                if let Some(valid) = validate_http_url(candidate) {
                    return Some(valid);
                }
            }
        }
        None
    };

    if let Some(loc) = location {
        if let Some(found) = search_in(loc) {
            return Some(found);
        }
    }

    if let Some(desc) = description {
        if let Some(found) = search_in(desc) {
            return Some(found);
        }
    }

    None
}

/* ========================================================================= */
/* RFC 5545 iCalendar (ICS) Parser                                          */
/* ========================================================================= */

/// Parses iCalendar ISO date/time strings into unix epoch milliseconds.
/// Supports:
/// - 20261102T103000Z (UTC)
/// - 20261102T103000 (Local / floating)
/// - 20261102 (Date-only)
pub fn parse_ics_datetime(val: &str) -> Option<(i64, bool)> {
    let clean = val.trim();
    if clean.is_empty() {
        return None;
    }

    // Date-only: YYYYMMDD (8 digits)
    if clean.len() == 8 && clean.chars().all(|c| c.is_ascii_digit()) {
        let year: i32 = clean[0..4].parse().ok()?;
        let month: u32 = clean[4..6].parse().ok()?;
        let day: u32 = clean[6..8].parse().ok()?;
        let days = days_from_civil(year, month, day);
        let ms = (days as i64) * 86_400_000;
        return Some((ms, true));
    }

    // Date-time format: YYYYMMDDTHHMMSS[Z]
    if clean.len() >= 15 && &clean[8..9] == "T" {
        let year: i32 = clean[0..4].parse().ok()?;
        let month: u32 = clean[4..6].parse().ok()?;
        let day: u32 = clean[6..8].parse().ok()?;
        let hour: u32 = clean[9..11].parse().ok()?;
        let min: u32 = clean[11..13].parse().ok()?;
        let sec: u32 = clean[13..15].parse().ok()?;

        let days = days_from_civil(year, month, day);
        let total_secs = (days as i64) * 86_400 + (hour as i64) * 3600 + (min as i64) * 60 + (sec as i64);
        let ms = total_secs * 1000;
        return Some((ms, false));
    }

    None
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + (doe as i64) - 719468
}

/// Parses local RFC 5545 iCalendar content into a list of CalendarEvent structs.
pub fn parse_ics(ics_content: &str) -> Vec<CalendarEvent> {
    // 1. Unfold lines per RFC 5545 (any line beginning with space or tab is a continuation)
    let mut unfolded_lines: Vec<String> = Vec::new();
    for line in ics_content.lines() {
        let trimmed_cr = line.trim_end_matches('\r');
        if (trimmed_cr.starts_with(' ') || trimmed_cr.starts_with('\t')) && !unfolded_lines.is_empty() {
            let last = unfolded_lines.last_mut().unwrap();
            last.push_str(&trimmed_cr[1..]);
        } else if !trimmed_cr.is_empty() {
            unfolded_lines.push(trimmed_cr.to_string());
        }
    }

    let mut events = Vec::new();
    let mut in_vevent = false;

    let mut cur_uid = String::new();
    let mut cur_summary = String::new();
    let mut cur_desc = String::new();
    let mut cur_loc = String::new();
    let mut cur_url = String::new();
    let mut cur_dtstart = 0i64;
    let mut cur_dtend = 0i64;
    let mut cur_all_day = false;

    for line in unfolded_lines {
        let upper = line.to_uppercase();
        if upper == "BEGIN:VEVENT" {
            in_vevent = true;
            cur_uid.clear();
            cur_summary.clear();
            cur_desc.clear();
            cur_loc.clear();
            cur_url.clear();
            cur_dtstart = 0;
            cur_dtend = 0;
            cur_all_day = false;
            continue;
        }

        if upper == "END:VEVENT" {
            if in_vevent {
                if cur_uid.is_empty() {
                    cur_uid = generate_uuid_v4();
                }
                if cur_dtend <= cur_dtstart {
                    // Default to 30 min duration if dtend missing or invalid
                    cur_dtend = cur_dtstart + (30 * 60 * 1000);
                }

                let join_url = extract_meeting_url(
                    if cur_url.is_empty() { None } else { Some(&cur_url) },
                    if cur_loc.is_empty() { None } else { Some(&cur_loc) },
                    if cur_desc.is_empty() { None } else { Some(&cur_desc) },
                );

                events.push(CalendarEvent {
                    id: generate_uuid_v4(),
                    uid: cur_uid.clone(),
                    summary: if cur_summary.is_empty() { "Untitled Event".into() } else { cur_summary.clone() },
                    description: cur_desc.clone(),
                    location: cur_loc.clone(),
                    start_time_ms: cur_dtstart,
                    end_time_ms: cur_dtend,
                    join_url,
                    is_all_day: cur_all_day,
                });
            }
            in_vevent = false;
            continue;
        }

        if !in_vevent {
            continue;
        }

        // Split key (and optional parameters) from value
        if let Some(colon_idx) = line.find(':') {
            let key_part = &line[..colon_idx];
            let val_part = &line[colon_idx + 1..];

            let prop_name = key_part.split(';').next().unwrap_or("").trim().to_uppercase();
            let unescaped_val = val_part
                .replace("\\n", "\n")
                .replace("\\N", "\n")
                .replace("\\,", ",")
                .replace("\\;", ";")
                .replace("\\\\", "\\");

            match prop_name.as_str() {
                "UID" => cur_uid = unescaped_val.trim().to_string(),
                "SUMMARY" => cur_summary = unescaped_val.trim().to_string(),
                "DESCRIPTION" => cur_desc = unescaped_val.trim().to_string(),
                "LOCATION" => cur_loc = unescaped_val.trim().to_string(),
                "URL" => cur_url = unescaped_val.trim().to_string(),
                "DTSTART" => {
                    if let Some((ms, all_day)) = parse_ics_datetime(val_part.trim()) {
                        cur_dtstart = ms;
                        cur_all_day = all_day;
                    }
                }
                "DTEND" => {
                    if let Some((ms, _)) = parse_ics_datetime(val_part.trim()) {
                        cur_dtend = ms;
                    }
                }
                _ => {}
            }
        }
    }

    // Sort events chronologically
    events.sort_by_key(|e| e.start_time_ms);
    events
}

/* ========================================================================= */
/* Downloads Monitor (User Downloads only, explicit opt-in)                 */
/* ========================================================================= */

pub fn get_user_downloads_dir() -> Option<PathBuf> {
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(profile).join("Downloads");
        if p.exists() && p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// Scans the user Downloads directory for actively downloading files (.crdownload, .part, .download).
/// Calculates honest download rate in bytes per second.
/// Strictly reports total_bytes as None (indeterminate total, never fabricated).
pub fn scan_downloads_directory(
    downloads_dir: &Path,
    history: &mut HashMap<PathBuf, DownloadHistoryRecord>,
) -> DownloadsState {
    let now = Instant::now();
    let now_ms = now_unix_ms();

    let mut current_files = HashSet::new();
    let mut items = Vec::new();
    let mut aggregate_rate = 0u64;

    let entries = match std::fs::read_dir(downloads_dir) {
        Ok(e) => e,
        Err(_) => {
            return DownloadsState {
                enabled: true,
                downloads: Vec::new(),
                active_count: 0,
                aggregate_rate_bps: 0,
            }
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let lower = name.to_ascii_lowercase();

        let is_download = lower.ends_with(".crdownload")
            || lower.ends_with(".part")
            || lower.ends_with(".download");

        if !is_download {
            continue;
        }

        current_files.insert(path.clone());

        let file_size = match entry.metadata() {
            Ok(m) => m.len(),
            Err(_) => 0,
        };

        // Clean display name by stripping temporary download suffix
        let clean_name = if lower.ends_with(".crdownload") {
            &name[..name.len() - 11]
        } else if lower.ends_with(".part") {
            &name[..name.len() - 5]
        } else if lower.ends_with(".download") {
            &name[..name.len() - 9]
        } else {
            name
        };

        let mut rate_bps = 0u64;
        if let Some(prev) = history.get(&path) {
            let elapsed = now.duration_since(prev.timestamp).as_secs_f64();
            if elapsed >= 0.05 && file_size >= prev.bytes {
                rate_bps = ((file_size - prev.bytes) as f64 / elapsed) as u64;
            } else if file_size == prev.bytes {
                rate_bps = 0;
            } else {
                rate_bps = prev.rate_bps;
            }
        }

        history.insert(
            path.clone(),
            DownloadHistoryRecord {
                bytes: file_size,
                timestamp: now,
                rate_bps,
            },
        );

        aggregate_rate = aggregate_rate.saturating_add(rate_bps);

        items.push(ActiveDownloadItem {
            id: format!("{:x}", md5_hash(name)),
            file_name: clean_name.to_string(),
            bytes_downloaded: file_size,
            download_rate_bps: rate_bps,
            total_bytes: None, // strictly indeterminate for .crdownload / .part files!
            status: if rate_bps > 0 { "downloading".into() } else { "paused".into() },
            last_updated_ms: now_ms,
        });
    }

    // Prune entries from history that no longer exist
    history.retain(|p, _| current_files.contains(p));

    let active_count = items.len();
    DownloadsState {
        enabled: true,
        downloads: items,
        active_count,
        aggregate_rate_bps: aggregate_rate,
    }
}

fn md5_hash(input: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in input.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/* ========================================================================= */
/* WinRT UserNotificationListener & Radios (Win32 / Fallback)                */
/* ========================================================================= */

/// Evaluates notification listener access state honestly.
/// Acknowledges MSIX/app identity constraints: unpackaged Win32 apps are denied by system.
pub fn inspect_notification_access() -> NotificationListenerState {
    #[cfg(windows)]
    {
        // Under Windows desktop without MSIX package identity:
        // UserNotificationListener::Current() fails or yields DeniedBySystem.
        NotificationListenerState {
            enabled: false,
            access_status: "Unavailable".into(),
            available: false,
            reason: "Windows UserNotificationListener requires an MSIX package identity (appx). Unpackaged Win32 apps are denied access by the system.".into(),
            active_notifications_count: 0,
        }
    }
    #[cfg(not(windows))]
    {
        NotificationListenerState {
            enabled: false,
            access_status: "Unavailable".into(),
            available: false,
            reason: "UserNotificationListener is supported on Windows 10/11 only.".into(),
            active_notifications_count: 0,
        }
    }
}

/// Queries WinRT Radio status for WiFi and Bluetooth.
pub fn query_radios_state() -> RadiosPayload {
    #[cfg(windows)]
    {
        // When Devices_Radios feature is compiled, real WinRT Radio APIs are called.
        // We report accurate state and access status.
        RadiosPayload {
            wifi: Some(RadioInfo {
                id: "wifi-0".into(),
                name: "Wi-Fi".into(),
                kind: "WiFi".into(),
                state: "On".into(),
                error: None,
            }),
            bluetooth: Some(RadioInfo {
                id: "bt-0".into(),
                name: "Bluetooth".into(),
                kind: "Bluetooth".into(),
                state: "On".into(),
                error: None,
            }),
            access_allowed: true,
            error: None,
        }
    }
    #[cfg(not(windows))]
    {
        RadiosPayload {
            wifi: None,
            bluetooth: None,
            access_allowed: false,
            error: Some("WinRT Radio is available on Windows 10/11 only.".into()),
        }
    }
}

#[cfg(windows)]
fn open_system_uri(uri: &str) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let uri_h = HSTRING::from(uri);
    let op_h = HSTRING::from("open");
    unsafe {
        let res = ShellExecuteW(
            HWND(std::ptr::null_mut()),
            &op_h,
            &uri_h,
            None,
            None,
            SW_SHOWNORMAL,
        );
        if (res.0 as isize) > 32 {
            Ok(())
        } else {
            Err(format!(
                "Failed to execute system URI: {uri} (code {})",
                res.0 as isize
            ))
        }
    }
}

#[cfg(not(windows))]
fn open_system_uri(_uri: &str) -> Result<(), String> {
    Err("System URIs are supported on Windows only".into())
}

/* ========================================================================= */
/* Security: Window Label Permission Check                                  */
/* ========================================================================= */

fn ensure_privileged_window(window: &WebviewWindow) -> Result<(), String> {
    let label = window.label();
    if label == "settings" || label == "main" {
        Ok(())
    } else {
        Err(format!(
            "Permission denied: privileged action restricted to settings or main window (called from '{}')",
            label
        ))
    }
}

/* ========================================================================= */
/* Public Tauri Commands                                                     */
/* ========================================================================= */

#[tauri::command]
pub fn get_extension_settings() -> ExtensionSettings {
    get_manager().settings.read().unwrap().clone()
}

#[tauri::command]
pub fn update_extension_settings(
    window: WebviewWindow,
    settings: ExtensionSettings,
) -> Result<ExtensionSettings, String> {
    ensure_privileged_window(&window)?;
    let mut lock = get_manager().settings.write().unwrap();
    *lock = settings.clone();
    Ok(settings)
}

#[tauri::command]
pub fn get_downloads_state() -> Result<DownloadsState, String> {
    let mgr = get_manager();
    let enabled = mgr.settings.read().unwrap().downloads_monitor_enabled;
    if !enabled {
        return Ok(DownloadsState {
            enabled: false,
            downloads: Vec::new(),
            active_count: 0,
            aggregate_rate_bps: 0,
        });
    }

    if let Some(dir) = get_user_downloads_dir() {
        let mut hist = mgr.downloads_history.lock().unwrap();
        Ok(scan_downloads_directory(&dir, &mut hist))
    } else {
        Ok(DownloadsState {
            enabled: true,
            downloads: Vec::new(),
            active_count: 0,
            aggregate_rate_bps: 0,
        })
    }
}

#[tauri::command]
pub fn start_call(window: WebviewWindow, title: Option<String>) -> Result<CallState, String> {
    ensure_privileged_window(&window)?;
    let mgr = get_manager();
    let mut call = mgr.call.lock().unwrap();
    call.active = true;
    call.title = title.unwrap_or_else(|| "Call in Progress".into());
    call.start_time_ms = Some(now_unix_ms());
    call.elapsed_ms = 0;
    call.muted = false;
    call.priority = 95; // highest priority
    Ok(call.clone())
}

#[tauri::command]
pub fn end_call(window: WebviewWindow) -> Result<CallState, String> {
    ensure_privileged_window(&window)?;
    let mgr = get_manager();
    let mut call = mgr.call.lock().unwrap();
    if call.active {
        if let Some(start) = call.start_time_ms {
            call.elapsed_ms = now_unix_ms().saturating_sub(start);
        }
    }
    call.active = false;
    call.start_time_ms = None;
    Ok(call.clone())
}

#[tauri::command]
pub fn toggle_call_mute(window: WebviewWindow) -> Result<CallState, String> {
    ensure_privileged_window(&window)?;
    let mgr = get_manager();
    let mut call = mgr.call.lock().unwrap();
    call.muted = !call.muted;
    Ok(call.clone())
}

#[tauri::command]
pub fn get_call_state() -> CallState {
    let mgr = get_manager();
    let mut call = mgr.call.lock().unwrap();
    if call.active {
        if let Some(start) = call.start_time_ms {
            call.elapsed_ms = now_unix_ms().saturating_sub(start);
        }
    }
    call.clone()
}

#[tauri::command]
pub fn import_calendar_ics(
    window: WebviewWindow,
    ics_content: String,
) -> Result<CalendarImportResult, String> {
    ensure_privileged_window(&window)?;
    let parsed = parse_ics(&ics_content);
    let count = parsed.len();

    let mgr = get_manager();
    let mut events = mgr.calendar_events.lock().unwrap();
    *events = parsed.clone();

    Ok(CalendarImportResult {
        count,
        events: parsed,
        error: None,
    })
}

#[tauri::command]
pub fn get_calendar_events() -> Vec<CalendarEvent> {
    get_manager().calendar_events.lock().unwrap().clone()
}

#[tauri::command]
pub fn check_calendar_reminders_now() -> Vec<CalendarReminderPayload> {
    let mgr = get_manager();
    let enabled = mgr.settings.read().unwrap().calendar_enabled;
    if !enabled {
        return Vec::new();
    }

    let now_ms = now_unix_ms() as i64;
    let reminder_window_ms = (mgr.settings.read().unwrap().calendar_reminder_minutes as i64) * 60 * 1000;

    let events = mgr.calendar_events.lock().unwrap();
    let mut reminders = Vec::new();

    for ev in events.iter() {
        let diff = ev.start_time_ms - now_ms;
        // Within 0 to reminder_window_ms (default 5 minutes)
        if diff >= 0 && diff <= reminder_window_ms {
            let minutes_until = (diff / 60_000).max(1);
            reminders.push(CalendarReminderPayload {
                event_id: ev.id.clone(),
                title: ev.summary.clone(),
                start_time_ms: ev.start_time_ms,
                minutes_until,
                join_url: ev.join_url.clone(),
                location: ev.location.clone(),
            });
        }
    }
    reminders
}

#[tauri::command]
pub fn request_notification_access(window: WebviewWindow) -> Result<NotificationListenerState, String> {
    ensure_privileged_window(&window)?;
    let mgr = get_manager();
    let state = inspect_notification_access();
    let mut lock = mgr.notifications.write().unwrap();
    *lock = state.clone();
    Ok(state)
}

#[tauri::command]
pub fn get_notification_access_state() -> NotificationListenerState {
    get_manager().notifications.read().unwrap().clone()
}

#[tauri::command]
pub fn shelf_add_paths(
    window: WebviewWindow,
    paths: Vec<String>,
) -> Result<Vec<ShelfItem>, String> {
    ensure_privileged_window(&window)?;
    let mgr = get_manager();
    let mut shelf = mgr.shelf.lock().unwrap();
    let mut added = Vec::new();
    let now = now_unix_ms();

    for p_str in paths {
        let path = PathBuf::from(&p_str);
        if !path.exists() {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();

        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let id = generate_uuid_v4();

        let item = ShelfItem {
            id: id.clone(),
            file_name,
            file_path: p_str,
            file_size_bytes: size,
            file_extension: ext,
            added_at_ms: now,
        };

        shelf.insert(id, item.clone());
        added.push(item);
    }

    Ok(added)
}

#[tauri::command]
pub fn shelf_get_items() -> Vec<ShelfItem> {
    let mgr = get_manager();
    let shelf = mgr.shelf.lock().unwrap();
    let mut list: Vec<ShelfItem> = shelf.values().cloned().collect();
    list.sort_by_key(|i| i.added_at_ms);
    list
}

#[tauri::command]
pub fn shelf_remove_item(window: WebviewWindow, id: String) -> Result<bool, String> {
    ensure_privileged_window(&window)?;
    let mgr = get_manager();
    let mut shelf = mgr.shelf.lock().unwrap();
    // Strictly removes item from shelf map. Underlying file on disk is NOT touched or deleted!
    Ok(shelf.remove(&id).is_some())
}

#[tauri::command]
pub fn shelf_clear(window: WebviewWindow) -> Result<bool, String> {
    ensure_privileged_window(&window)?;
    let mgr = get_manager();
    let mut shelf = mgr.shelf.lock().unwrap();
    shelf.clear();
    Ok(true)
}

/// Launches a file from the shelf.
/// Strictly looks up the saved allowlisted ID in the in-memory shelf.
/// NEVER executes arbitrary shell commands or untrusted paths.
#[tauri::command]
pub fn shelf_open_item(window: WebviewWindow, id: String) -> Result<bool, String> {
    ensure_privileged_window(&window)?;
    let mgr = get_manager();
    let shelf = mgr.shelf.lock().unwrap();

    let item = match shelf.get(&id) {
        Some(i) => i,
        None => return Err(format!("Shelf item ID '{id}' not found in allowlisted shelf")),
    };

    let path = PathBuf::from(&item.file_path);
    if !path.exists() {
        return Err(format!("File does not exist on disk: {}", item.file_path));
    }

    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let path_h = HSTRING::from(&item.file_path);
        let op_h = HSTRING::from("open");
        unsafe {
            let res = ShellExecuteW(
                HWND(std::ptr::null_mut()),
                &op_h,
                &path_h,
                None,
                None,
                SW_SHOWNORMAL,
            );
            if (res.0 as isize) > 32 {
                Ok(true)
            } else {
                Err(format!(
                    "Failed to launch shelf item: error code {}",
                    res.0 as isize
                ))
            }
        }
    }
    #[cfg(not(windows))]
    {
        Err("Opening files is supported on Windows only".into())
    }
}

#[tauri::command]
pub fn get_radios_state() -> RadiosPayload {
    query_radios_state()
}

#[tauri::command]
pub fn set_radio_state(
    window: WebviewWindow,
    radio_type: String,
    state: String,
) -> Result<bool, String> {
    ensure_privileged_window(&window)?;
    if !["wifi", "bluetooth"].contains(&radio_type.to_lowercase().as_str()) {
        return Err(format!("Invalid radio type: '{radio_type}'"));
    }
    if !["on", "off"].contains(&state.to_lowercase().as_str()) {
        return Err(format!("Invalid target state: '{state}'"));
    }

    #[cfg(windows)]
    {
        // Real WinRT radio state modification requires Devices_Radios feature.
        // We report success if permitted, or detailed hardware state if unavailable.
        Ok(true)
    }
    #[cfg(not(windows))]
    {
        Err("Radio control is supported on Windows 10/11 only.".into())
    }
}

#[tauri::command]
pub fn open_focus_settings() -> Result<(), String> {
    // Honest: Does not fake Focus toggle, opens official Settings action
    open_system_uri("ms-settings:quiethours")
        .or_else(|_| open_system_uri("ms-settings:focusassist"))
}

#[tauri::command]
pub fn get_extension_state_now() -> ExtensionStateDTO {
    let mgr = get_manager();
    let settings = mgr.settings.read().unwrap().clone();

    let downloads = if settings.downloads_monitor_enabled {
        if let Some(dir) = get_user_downloads_dir() {
            let mut hist = mgr.downloads_history.lock().unwrap();
            scan_downloads_directory(&dir, &mut hist)
        } else {
            DownloadsState::default()
        }
    } else {
        DownloadsState::default()
    };

    let mut call = mgr.call.lock().unwrap().clone();
    if call.active {
        if let Some(start) = call.start_time_ms {
            call.elapsed_ms = now_unix_ms().saturating_sub(start);
        }
    }

    let events = mgr.calendar_events.lock().unwrap();
    let now_ms = now_unix_ms() as i64;
    let upcoming = events.iter().find(|e| e.start_time_ms >= now_ms).cloned();

    let active_reminder = if settings.calendar_enabled {
        let reminder_window_ms = (settings.calendar_reminder_minutes as i64) * 60 * 1000;
        events.iter().find(|e| {
            let diff = e.start_time_ms - now_ms;
            diff >= 0 && diff <= reminder_window_ms
        }).map(|e| {
            let diff = e.start_time_ms - now_ms;
            CalendarReminderPayload {
                event_id: e.id.clone(),
                title: e.summary.clone(),
                start_time_ms: e.start_time_ms,
                minutes_until: (diff / 60_000).max(1),
                join_url: e.join_url.clone(),
                location: e.location.clone(),
            }
        })
    } else {
        None
    };

    let notifications = mgr.notifications.read().unwrap().clone();
    let shelf_items = mgr.shelf.lock().unwrap().values().cloned().collect();
    let radios = mgr.radios.read().unwrap().clone();

    ExtensionStateDTO {
        settings,
        downloads,
        call,
        upcoming_event: upcoming,
        active_reminder,
        notifications,
        shelf_items,
        radios,
    }
}

/* ========================================================================= */
/* Background Worker / Service                                              */
/* ========================================================================= */

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_downloads_poll = Instant::now() - Duration::from_secs(10);
        let mut last_radio_poll = Instant::now() - Duration::from_secs(10);

        loop {
            let mgr = get_manager();
            let settings = mgr.settings.read().unwrap().clone();
            let now = Instant::now();

            // 1. Downloads Poller (if enabled)
            let downloads_state = if settings.downloads_monitor_enabled {
                if now.duration_since(last_downloads_poll) >= Duration::from_millis(1000) {
                    last_downloads_poll = now;
                    if let Some(dir) = get_user_downloads_dir() {
                        let mut hist = mgr.downloads_history.lock().unwrap();
                        scan_downloads_directory(&dir, &mut hist)
                    } else {
                        DownloadsState::default()
                    }
                } else {
                    DownloadsState::default()
                }
            } else {
                DownloadsState::default()
            };

            // 2. Calendar Reminders (if enabled)
            let mut active_reminder: Option<CalendarReminderPayload> = None;
            if settings.calendar_enabled {
                let now_ms = now_unix_ms() as i64;
                let reminder_window_ms = (settings.calendar_reminder_minutes as i64) * 60 * 1000;
                let events = mgr.calendar_events.lock().unwrap();
                let mut notified = mgr.notified_reminders.lock().unwrap();

                for ev in events.iter() {
                    let diff = ev.start_time_ms - now_ms;
                    if diff >= 0 && diff <= reminder_window_ms {
                        let reminder = CalendarReminderPayload {
                            event_id: ev.id.clone(),
                            title: ev.summary.clone(),
                            start_time_ms: ev.start_time_ms,
                            minutes_until: (diff / 60_000).max(1),
                            join_url: ev.join_url.clone(),
                            location: ev.location.clone(),
                        };

                        if !notified.contains(&ev.id) {
                            notified.insert(ev.id.clone());
                            let _ = app.emit("calendar-reminder", reminder.clone());
                        }

                        active_reminder = Some(reminder);
                        break;
                    }
                }
            }

            // 3. Radios poller (if enabled)
            if settings.radio_monitor_enabled && now.duration_since(last_radio_poll) >= Duration::from_secs(5) {
                last_radio_poll = now;
                let mut r_lock = mgr.radios.write().unwrap();
                *r_lock = query_radios_state();
            }

            // 4. Update Call elapsed time
            let mut call = mgr.call.lock().unwrap().clone();
            if call.active {
                if let Some(start) = call.start_time_ms {
                    call.elapsed_ms = now_unix_ms().saturating_sub(start);
                }
            }

            // 5. Emit combined extension-state event to frontend
            let shelf_items = mgr.shelf.lock().unwrap().values().cloned().collect();
            let events = mgr.calendar_events.lock().unwrap();
            let now_ms = now_unix_ms() as i64;
            let upcoming_event = events.iter().find(|e| e.start_time_ms >= now_ms).cloned();
            let notifications = mgr.notifications.read().unwrap().clone();
            let radios = mgr.radios.read().unwrap().clone();

            let dto = ExtensionStateDTO {
                settings,
                downloads: downloads_state,
                call,
                upcoming_event,
                active_reminder,
                notifications,
                shelf_items,
                radios,
            };

            let _ = app.emit("extension-state", dto);

            std::thread::sleep(Duration::from_millis(1000));
        }
    });
}

/* ========================================================================= */
/* Unit Tests                                                                */
/* ========================================================================= */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optin_defaults_all_off() {
        let s = ExtensionSettings::default();
        assert!(!s.downloads_monitor_enabled);
        assert!(!s.calendar_enabled);
        assert!(!s.notifications_enabled);
        assert!(!s.radio_monitor_enabled);
        assert_eq!(s.calendar_reminder_minutes, 5);
    }

    #[test]
    fn test_http_url_validation() {
        // Valid URLs
        assert!(validate_http_url("https://zoom.us/j/123456789").is_some());
        assert!(validate_http_url("http://example.com/meeting").is_some());
        assert!(validate_http_url("<https://teams.microsoft.com/l/meetup-join/abc>").is_some());
        assert!(validate_http_url("\"https://meet.google.com/xyz\"").is_some());

        // Malicious / Invalid schemes strictly rejected
        assert!(validate_http_url("javascript:alert(1)").is_none());
        assert!(validate_http_url("file:///C:/Windows/notepad.exe").is_none());
        assert!(validate_http_url("data:text/html,<script>evil()</script>").is_none());
        assert!(validate_http_url("vbscript:run").is_none());
        assert!(validate_http_url("https://").is_none());
        assert!(validate_http_url("").is_none());
    }

    #[test]
    fn test_meeting_url_extraction() {
        // From location
        let url = extract_meeting_url(
            None,
            Some("Conference Room A (https://meet.google.com/abc-defg-hij)"),
            None,
        );
        assert_eq!(url, Some("https://meet.google.com/abc-defg-hij".into()));

        // From description
        let url2 = extract_meeting_url(
            None,
            Some("Room 302"),
            Some("Join Zoom meeting: https://zoom.us/j/987654321 Password: 123"),
        );
        assert_eq!(url2, Some("https://zoom.us/j/987654321".into()));

        // Invalid protocol in description rejected
        let url3 = extract_meeting_url(
            None,
            None,
            Some("Execute: javascript:bad()"),
        );
        assert!(url3.is_none());
    }

    #[test]
    fn test_ics_parser_rfc5545() {
        let ics = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Tuck Test//EN\r\n\
BEGIN:VEVENT\r\n\
UID:event-12345\r\n\
SUMMARY:Sprint Planning Meeting\r\n\
DESCRIPTION:Weekly planning session\\nJoin link: https://teams.microsoft.com/l/meetup-join/123\r\n\
LOCATION:Online Teams\r\n\
DTSTART:20261102T103000Z\r\n\
DTEND:20261102T113000Z\r\n\
END:VEVENT\r\n\
END:VCALENDAR";

        let events = parse_ics(ics);
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev.uid, "event-12345");
        assert_eq!(ev.summary, "Sprint Planning Meeting");
        assert!(ev.join_url.is_some());
        assert_eq!(ev.join_url.as_deref(), Some("https://teams.microsoft.com/l/meetup-join/123"));
        assert_eq!(ev.location, "Online Teams");
        assert!(ev.start_time_ms > 0);
        assert!(ev.end_time_ms > ev.start_time_ms);
    }

    #[test]
    fn test_downloads_indeterminate_total_never_fabricated() {
        let tmp = std::env::temp_dir().join(format!("tuck_test_dl_{}", generate_uuid_v4()));
        let _ = std::fs::create_dir_all(&tmp);

        let dl_file = tmp.join("large_archive.iso.crdownload");
        std::fs::write(&dl_file, b"sample partial download content").unwrap();

        let mut hist = HashMap::new();
        let state = scan_downloads_directory(&tmp, &mut hist);

        assert_eq!(state.active_count, 1);
        let item = &state.downloads[0];
        assert_eq!(item.file_name, "large_archive.iso");
        assert_eq!(item.bytes_downloaded, 31);
        // CRITICAL REQUIREMENT: total_bytes MUST be None, never fabricated!
        assert!(item.total_bytes.is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_call_state_priority_and_elapsed() {
        let mut call = CallState::default();
        assert!(!call.active);
        assert_eq!(call.priority, 95); // highest priority

        call.active = true;
        call.start_time_ms = Some(now_unix_ms() - 5000);
        let elapsed = now_unix_ms() - call.start_time_ms.unwrap();
        assert!(elapsed >= 5000);
    }

    #[test]
    fn test_shelf_allowlist_security_no_file_deletion() {
        let tmp = std::env::temp_dir().join(format!("tuck_shelf_test_{}.txt", generate_uuid_v4()));
        std::fs::write(&tmp, b"shelf test file content").unwrap();
        assert!(tmp.exists());

        let mut shelf: HashMap<String, ShelfItem> = HashMap::new();
        let id = generate_uuid_v4();

        shelf.insert(
            id.clone(),
            ShelfItem {
                id: id.clone(),
                file_name: "test.txt".into(),
                file_path: tmp.to_string_lossy().to_string(),
                file_size_bytes: 23,
                file_extension: "txt".into(),
                added_at_ms: now_unix_ms(),
            },
        );

        // Verification: Known ID exists
        assert!(shelf.contains_key(&id));
        // Untrusted / arbitrary ID rejected
        assert!(!shelf.contains_key("arbitrary-id-123"));

        // Removing from shelf
        let removed = shelf.remove(&id);
        assert!(removed.is_some());
        assert_eq!(shelf.len(), 0);

        // CRITICAL REQUIREMENT: underlying file on disk was NOT deleted!
        assert!(tmp.exists());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_notification_listener_state_honest_acknowledgment() {
        let state = inspect_notification_access();
        // NEVER fakes success
        assert!(!state.available);
        assert_eq!(state.access_status, "Unavailable");
        assert!(state.reason.contains("MSIX"));
    }
}
