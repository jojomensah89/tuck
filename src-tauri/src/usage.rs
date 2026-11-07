//! Tuck subscription usage backend. Reads local Claude Code and Codex usage data.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::Read,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime},
};
use tauri::{AppHandle, Emitter, Runtime};

// ============================================================================
// Constants & Configuration
// ============================================================================

/// Claude statusline data is considered stale if unmodified for over 600 seconds (10 minutes).
pub const STALE_THRESHOLD_SECS: u64 = 600;
/// A status file older than the weekly window says Claude Code is signed out or gone.
pub const CONNECTED_THRESHOLD_SECS: u64 = 7 * 24 * 60 * 60;

/// Bounded file read size limit (2 MB) to prevent denial of service or unbounded parsing.
pub const MAX_STATUS_FILE_BYTES: u64 = 2 * 1024 * 1024;

const FIVE_HOUR_PTR: &str = "/rate_limits/five_hour";
const SEVEN_DAY_PTR: &str = "/rate_limits/seven_day";

// ============================================================================
// Data Transfer Objects (DTO)
// ============================================================================

/// Meter representation transferred to the frontend over Tauri IPC.
///
/// Follows camelCase naming convention:
/// `id`, `provider`, `label`, `usedPct`, `spentUsd`, `resetsAt`, `source`, `stale`, `status`, `error`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Meter {
    /// Unique subscription meter identifier.
    pub id: String,
    /// Provider category.
    pub provider: String,
    /// Display label for UI representation.
    pub label: String,
    /// Percentage of subscription quota used (0..100).
    pub used_pct: Option<f64>,
    /// Reserved for compatibility; subscription meters do not report spend.
    pub spent_usd: Option<f64>,
    /// Expected reset time as a Unix timestamp in seconds.
    pub resets_at: Option<i64>,
    /// Source provenance.
    pub source: String,
    /// Indicates whether the data is considered stale (>600s since update, or failed refresh).
    pub stale: bool,
    /// Optional additive status field ("ok", "stale", "unavailable", "error", "unconfigured").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Optional additive error field with sanitized/redacted details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ============================================================================
// Claude Statusline Parser & File Reader
// ============================================================================

/// Strips UTF-8 Byte Order Mark (BOM: `[0xEF, 0xBB, 0xBF]`) if present.
pub fn strip_bom(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    }
}

/// Parses a Unix timestamp in seconds from either a JSON integer or an RFC 3339 string.
pub fn parse_unix_timestamp(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        return Some(n);
    }
    if let Some(s) = v.as_str() {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return Some(dt.timestamp());
        }
        if let Ok(n) = s.parse::<i64>() {
            return Some(n);
        }
    }
    None
}

/// Extracts a finite floating-point percentage (0..100) from a JSON value.
/// Rejects null, non-numeric, NaN, and infinite values.
pub fn extract_percentage(val: Option<&Value>) -> Option<f64> {
    let v = val?;
    let num = v.as_f64()?;
    if num.is_finite() && (0.0..=100.0).contains(&num) {
        Some(num)
    } else {
        None
    }
}

/// Parses Claude Code session statusline JSON content into deterministic Meters.
///
/// Verifies official `rate_limits` schema (`/rate_limits/five_hour`, `/rate_limits/seven_day`).
/// Missing or malformed limits are explicitly marked unavailable rather than fabricated as zero.
pub fn parse_claude_status_content(
    content_bytes: &[u8],
    modified_time: Option<SystemTime>,
) -> Vec<Meter> {
    let clean_bytes = strip_bom(content_bytes);
    let is_stale = match modified_time {
        Some(t) => match SystemTime::now().duration_since(t) {
            Ok(d) => d.as_secs() > STALE_THRESHOLD_SECS,
            Err(_) => true,
        },
        None => true,
    };

    let parsed: Result<Value, _> = serde_json::from_slice(clean_bytes);
    match parsed {
        Ok(v) => {
            let mut out = Vec::with_capacity(2);
            for (id, label, ptr) in [
                ("claude-5h", "Claude session (5 h)", FIVE_HOUR_PTR),
                ("claude-week", "Claude weekly", SEVEN_DAY_PTR),
            ] {
                if let Some(target) = v.pointer(ptr) {
                    if target.is_null() {
                        out.push(Meter {
                            id: id.to_string(),
                            provider: "claude".to_string(),
                            label: label.to_string(),
                            used_pct: None,
                            spent_usd: None,
                            resets_at: None,
                            source: "official".to_string(),
                            stale: is_stale,
                            status: Some("unavailable".to_string()),
                            error: None,
                        });
                        continue;
                    }

                    let used_pct = extract_percentage(target.get("used_percentage"));
                    let resets_at = target.get("resets_at").and_then(parse_unix_timestamp);

                    let status = if used_pct.is_none() || resets_at.is_none() {
                        Some("unavailable".to_string())
                    } else if is_stale {
                        Some("stale".to_string())
                    } else {
                        Some("ok".to_string())
                    };

                    out.push(Meter {
                        id: id.to_string(),
                        provider: "claude".to_string(),
                        label: label.to_string(),
                        used_pct,
                        spent_usd: None,
                        resets_at,
                        source: "official".to_string(),
                        stale: is_stale,
                        status,
                        error: None,
                    });
                } else {
                    out.push(Meter {
                        id: id.to_string(),
                        provider: "claude".to_string(),
                        label: label.to_string(),
                        used_pct: None,
                        spent_usd: None,
                        resets_at: None,
                        source: "official".to_string(),
                        stale: is_stale,
                        status: Some("unavailable".to_string()),
                        error: None,
                    });
                }
            }
            out
        }
        Err(err) => {
            let _ = err;
            let err_msg = "Malformed JSON in status file".to_string();
            vec![
                Meter {
                    id: "claude-5h".to_string(),
                    provider: "claude".to_string(),
                    label: "Claude session (5 h)".to_string(),
                    used_pct: None,
                    spent_usd: None,
                    resets_at: None,
                    source: "official".to_string(),
                    stale: true,
                    status: Some("unavailable".to_string()),
                    error: Some(err_msg.clone()),
                },
                Meter {
                    id: "claude-week".to_string(),
                    provider: "claude".to_string(),
                    label: "Claude weekly".to_string(),
                    used_pct: None,
                    spent_usd: None,
                    resets_at: None,
                    source: "official".to_string(),
                    stale: true,
                    status: Some("unavailable".to_string()),
                    error: Some(err_msg),
                },
            ]
        }
    }
}

/// Locates the Claude statusline file:
/// 1. Preferred path: `%APPDATA%\Tuck\claude-status.json`
/// 2. Guide fallback: `%APPDATA%\Notch\claude-status.json`
pub fn resolve_claude_status_path() -> (Option<PathBuf>, bool) {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let tuck_path = PathBuf::from(&appdata)
            .join("Tuck")
            .join("claude-status.json");
        if tuck_path.exists() {
            return (Some(tuck_path), true);
        }
        let notch_path = PathBuf::from(&appdata)
            .join("Notch")
            .join("claude-status.json");
        if notch_path.exists() {
            return (Some(notch_path), true);
        }
        return (Some(tuck_path), false);
    }
    (None, false)
}

/// Fresher files show, stale if need be; a file with an unknown or future time gets the
/// benefit of the doubt.
fn status_file_connected(modified: Option<SystemTime>, now: SystemTime) -> bool {
    modified
        .and_then(|time| now.duration_since(time).ok())
        .is_none_or(|age| age.as_secs() <= CONNECTED_THRESHOLD_SECS)
}

/// Reads and parses Claude Code statusline meters from disk.
pub fn claude_meters() -> Vec<Meter> {
    let (target_path, exists) = resolve_claude_status_path();
    let Some(path) = target_path else {
        return Vec::new();
    };

    if !exists {
        return Vec::new();
    }

    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return unavailable_claude_meters(&format!("Unable to read metadata: {}", e)),
    };
    if !status_file_connected(meta.modified().ok(), SystemTime::now()) {
        return Vec::new();
    }

    if meta.len() > MAX_STATUS_FILE_BYTES {
        return unavailable_claude_meters("Status file exceeds size limit (2MB)");
    }

    let modified = meta.modified().ok();
    let bytes = (|| -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?
            .take(MAX_STATUS_FILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    })();
    let bytes = match bytes {
        Ok(bytes) if bytes.len() as u64 <= MAX_STATUS_FILE_BYTES => bytes,
        _ => return unavailable_claude_meters("Status file unavailable or exceeds size limit"),
    };

    parse_claude_status_content(&bytes, modified)
}

fn unavailable_claude_meters(reason: &str) -> Vec<Meter> {
    vec![
        Meter {
            id: "claude-5h".to_string(),
            provider: "claude".to_string(),
            label: "Claude session (5 h)".to_string(),
            used_pct: None,
            spent_usd: None,
            resets_at: None,
            source: "official".to_string(),
            stale: true,
            status: Some("unavailable".to_string()),
            error: Some(reason.to_string()),
        },
        Meter {
            id: "claude-week".to_string(),
            provider: "claude".to_string(),
            label: "Claude weekly".to_string(),
            used_pct: None,
            spent_usd: None,
            resets_at: None,
            source: "official".to_string(),
            stale: true,
            status: Some("unavailable".to_string()),
            error: Some(reason.to_string()),
        },
    ]
}

// ============================================================================
// State Cache & Lifecycle Management
// ============================================================================

#[derive(Debug, Default)]
struct SharedState {
    claude: Vec<Meter>,
}
static CACHE: OnceLock<Arc<Mutex<SharedState>>> = OnceLock::new();
static MANUAL_BUSY: AtomicBool = AtomicBool::new(false);
static USAGE_WAKE: (Mutex<()>, std::sync::Condvar) = (Mutex::new(()), std::sync::Condvar::new());
static REQUESTED: AtomicU64 = AtomicU64::new(0);
static COMPLETED: AtomicU64 = AtomicU64::new(0);
fn get_cache() -> &'static Arc<Mutex<SharedState>> {
    CACHE.get_or_init(|| Arc::new(Mutex::new(SharedState::default())))
}
pub fn current_snapshot() -> Vec<Meter> {
    let state = get_cache().lock().unwrap_or_else(|p| p.into_inner());
    let mut meters = if crate::installed_usage::provider_enabled("claude") {
        state.claude.clone()
    } else {
        Vec::new()
    };
    let installed = crate::installed_usage::snapshot();
    if installed
        .iter()
        .any(|meter| meter.id.starts_with("claude-installed-") && meter.used_pct.is_some())
    {
        meters.retain(|meter| meter.provider != "claude");
    }
    meters.extend(installed);
    meters
}
#[tauri::command]
pub fn usage_now() -> Vec<Meter> {
    current_snapshot()
}
/// Refresh enabled subscription readers, including the opt-in Antigravity credential-file exception.
#[tauri::command]
pub async fn refresh_usage() -> Result<Vec<Meter>, String> {
    if MANUAL_BUSY.swap(true, Ordering::SeqCst) {
        return Err("Usage refresh already pending".into());
    }
    tauri::async_runtime::spawn_blocking(|| {
        struct BusyGuard;
        impl Drop for BusyGuard {
            fn drop(&mut self) {
                MANUAL_BUSY.store(false, Ordering::SeqCst);
            }
        }
        let _guard = BusyGuard;
        let ticket = {
            let _gate = USAGE_WAKE.0.lock().unwrap_or_else(|p| p.into_inner());
            let ticket = REQUESTED.fetch_add(1, Ordering::SeqCst) + 1;
            USAGE_WAKE.1.notify_one();
            ticket
        };
        let installed = crate::installed_usage::request_manual_refresh();
        let started = Instant::now();
        while COMPLETED.load(Ordering::SeqCst) < ticket
            || !crate::installed_usage::manual_refresh_finished(installed)
        {
            if started.elapsed() > Duration::from_secs(90) {
                return Err("Usage refresh timed out; previous readings retained".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(current_snapshot())
    })
    .await
    .map_err(|_| "Usage refresh worker failed".to_string())?
}
pub fn start<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || {
        let mut last_file = Instant::now() - Duration::from_secs(60);
        let mut emitted = None;
        loop {
            let manual_ticket = REQUESTED.load(Ordering::SeqCst);
            if manual_ticket > COMPLETED.load(Ordering::SeqCst)
                || last_file.elapsed() >= Duration::from_secs(60)
            {
                let mut claude = if crate::installed_usage::provider_enabled("claude") {
                    claude_meters()
                } else {
                    Vec::new()
                };
                let mut cache = get_cache().lock().unwrap_or_else(|p| p.into_inner());
                for meter in &mut claude {
                    if meter.stale && meter.used_pct.is_none() {
                        if let Some(previous) = cache.claude.iter().find(|old| old.id == meter.id) {
                            meter.used_pct = previous.used_pct;
                        }
                    }
                }
                cache.claude = claude;
                last_file = Instant::now();
                COMPLETED.store(manual_ticket, Ordering::SeqCst);
            }
            let snapshot = current_snapshot();
            if emitted.as_ref() != Some(&snapshot) {
                let _ = app.emit("usage", &snapshot);
                emitted = Some(snapshot);
            }
            let gate = USAGE_WAKE.0.lock().unwrap_or_else(|p| p.into_inner());
            let due = Duration::from_secs(60).saturating_sub(last_file.elapsed());
            let _guard = USAGE_WAKE
                .1
                .wait_timeout_while(gate, due, |_| {
                    REQUESTED.load(Ordering::SeqCst) <= COMPLETED.load(Ordering::SeqCst)
                })
                .unwrap_or_else(|p| p.into_inner());
        }
    });
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn strips_bom_and_parses_subscription_meters() {
        let bytes = b"\xef\xbb\xbf{\"rate_limits\":{\"five_hour\":{\"used_percentage\":42.5,\"resets_at\":\"2026-11-06T12:00:00Z\"},\"seven_day\":{\"used_percentage\":12}}}";
        let meters = parse_claude_status_content(bytes, Some(SystemTime::now()));
        assert_eq!(meters[0].used_pct, Some(42.5));
        assert_eq!(meters[0].status.as_deref(), Some("ok"));
        assert_eq!(meters[1].used_pct, Some(12.0));
    }

    #[test]
    fn malformed_or_nonfinite_subscription_data_is_unavailable() {
        for bytes in [
            b"not json".as_slice(),
            b"{\"rate_limits\":null}",
            b"{\"rate_limits\":{\"five_hour\":{\"used_percentage\":101}}}",
        ] {
            let meters = parse_claude_status_content(bytes, Some(SystemTime::now()));
            assert!(meters
                .iter()
                .all(|meter| meter.status.as_deref() == Some("unavailable")));
        }
    }

    #[test]
    fn status_files_older_than_weekly_window_are_disconnected() {
        let now = SystemTime::now();
        assert!(status_file_connected(
            Some(now - Duration::from_secs(CONNECTED_THRESHOLD_SECS)),
            now
        ));
        assert!(!status_file_connected(
            Some(now - Duration::from_secs(CONNECTED_THRESHOLD_SECS + 1)),
            now
        ));
    }

    #[test]
    fn meter_serializes_with_frontend_casing() {
        let meter = Meter {
            id: "claude-5h".into(),
            provider: "claude".into(),
            label: "Claude session".into(),
            used_pct: Some(42.5),
            spent_usd: None,
            resets_at: Some(1700000000),
            source: "official".into(),
            stale: false,
            status: Some("ok".into()),
            error: None,
        };
        let json = serde_json::to_string(&meter).unwrap();
        assert!(json.contains("\"usedPct\":42.5"));
        assert!(json.contains("\"resetsAt\":1700000000"));
        assert!(!json.contains("used_pct"));
    }
}
