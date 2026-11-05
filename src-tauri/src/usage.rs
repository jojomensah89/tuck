//! Tuck AI Usage Backend
//!
//! Provides background polling and non-blocking IPC snapshots for AI usage meters:
//! - Claude Pro / Max session & weekly rate limits (fed by Claude Code statusline script)
//! - Anthropic Admin API month-to-date spend (`cost_report` endpoint)
//! - OpenAI Organization Costs API month-to-date spend (`organization/costs` endpoint)
//!
//! Credentials are stored exclusively in the Windows Credential Manager under the `tuck` service.

use chrono::{Datelike, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    io::Read,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime},
};
use tauri::{AppHandle, Emitter, Runtime};

// ============================================================================
// Constants & Configuration
// ============================================================================

/// Windows Credential Manager service name for Tuck.
pub const KEYRING_SERVICE: &str = "tuck";

/// Allowlist of permitted secret names in Credential Manager.
pub const ALLOWED_SECRET_NAMES: &[&str] = &["anthropic-admin", "openai-admin"];

/// Claude statusline data is considered stale if unmodified for over 600 seconds (10 minutes).
pub const STALE_THRESHOLD_SECS: u64 = 600;
/// A status file older than the weekly window says Claude Code is signed out or gone.
pub const CONNECTED_THRESHOLD_SECS: u64 = 7 * 24 * 60 * 60;

/// Bounded file read size limit (2 MB) to prevent denial of service or unbounded parsing.
pub const MAX_STATUS_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// HTTP client timeout for API requests (15 seconds).
pub const HTTP_TIMEOUT_SECS: u64 = 15;

/// Maximum number of pagination pages to follow per API request to prevent infinite loops.
pub const MAX_PAGINATION_PAGES: usize = 50;

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
    /// Unique meter identifier (e.g. "claude-5h", "claude-week", "anthropic-api", "openai-api").
    pub id: String,
    /// Provider category ("claude", "anthropic-api", "openai-api").
    pub provider: String,
    /// Display label for UI representation.
    pub label: String,
    /// Percentage of quota used (0..100) when reported by the source.
    /// `None` indicates percentage data is unavailable or not applicable (e.g. pay-per-token API).
    pub used_pct: Option<f64>,
    /// Month-to-date spending in USD for pay-per-token API providers.
    /// `None` indicates spending data is unavailable or not applicable (e.g. subscription tiers).
    pub spent_usd: Option<f64>,
    /// Expected reset time as a Unix timestamp in seconds.
    pub resets_at: Option<i64>,
    /// Source provenance ("official" for statusline, "api" for direct endpoints).
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
// Security & Redaction
// ============================================================================

/// Validates whether a secret name is in the allowed whitelist.
pub fn is_allowed_secret_name(name: &str) -> bool {
    ALLOWED_SECRET_NAMES.contains(&name.trim())
}

/// Sanitizes error messages to prevent accidental leakage of API keys or auth tokens.
pub fn sanitize_error(input: &str) -> String {
    // Raw OS/network/provider error strings can contain credentials or PII.
    // Expose a fixed diagnostic instead of attempting lossy token matching.
    if input.is_empty() {
        String::new()
    } else {
        "Provider operation failed; details withheld to protect credentials".into()
    }
}

// ============================================================================
// Windows Credential Manager (Keyring) Integration
// ============================================================================

/// Reads a write-only secret from Windows Credential Manager.
/// Secrets are never logged, serialized to disk, or returned to the client.
pub fn get_secret(name: &str) -> Result<Option<String>, String> {
    let name = name.trim();
    if !is_allowed_secret_name(name) {
        return Err("Credential name not allowed".into());
    }
    let entry =
        keyring::Entry::new(KEYRING_SERVICE, name).map_err(|_| "Credential Manager unavailable")?;
    match entry.get_password() {
        Ok(value) => Ok(if value.trim().is_empty() {
            None
        } else {
            Some(value.trim().to_owned())
        }),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("Credential Manager read failed".into()),
    }
}

/// Stores or deletes an administrative secret in Windows Credential Manager.
///
/// An empty or whitespace-only value explicitly deletes the stored credential.
/// Frontend can pass empty string to delete or non-empty string to update.
#[tauri::command]
fn set_secret_blocking(name: String, value: String) -> Result<(), String> {
    let name_trimmed = name.trim();
    if !is_allowed_secret_name(name_trimmed) {
        return Err("Credential name is not allowed".into());
    }

    let entry = keyring::Entry::new(KEYRING_SERVICE, name_trimmed)
        .map_err(|e| format!("Keyring error: {}", sanitize_error(&e.to_string())))?;

    let val_trimmed = value.trim();
    if val_trimmed.is_empty() {
        // Explicit deletion
        match entry.delete_credential() {
            Ok(()) => {
                invalidate_api_slot(name_trimmed, false);
                Ok(())
            }
            Err(keyring::Error::NoEntry) => {
                invalidate_api_slot(name_trimmed, false);
                Ok(())
            }
            Err(e) => Err(format!(
                "Keyring deletion error: {}",
                sanitize_error(&e.to_string())
            )),
        }
    } else {
        entry
            .set_password(val_trimmed)
            .map_err(|e| format!("Keyring save error: {}", sanitize_error(&e.to_string())))?;
        invalidate_api_slot(name_trimmed, true);
        Ok(())
    }
}

#[tauri::command]
pub async fn set_secret(name: String, value: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || set_secret_blocking(name, value))
        .await
        .map_err(|_| "Credential worker failed".to_string())?
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
// API Parsers (Anthropic & OpenAI)
// ============================================================================

/// Parses a single page from the Anthropic Organization Cost Report endpoint.
///
/// In Anthropic's official `cost_report` API, the `amount` field represents **cents**
/// as a decimal string (e.g. `"1250.50"`). This function parses and converts to USD (`cents / 100.0`).
///
/// Returns `(page_usd, has_more, next_page_cursor)`.
pub fn parse_anthropic_cost_page(v: &Value) -> Result<(f64, bool, Option<String>), String> {
    parse_cost_page(v, true)
}
pub fn parse_openai_costs_page(v: &Value) -> Result<(f64, bool, Option<String>), String> {
    parse_cost_page(v, false)
}
fn parse_cost_page(v: &Value, anthropic: bool) -> Result<(f64, bool, Option<String>), String> {
    let data = v
        .get("data")
        .and_then(Value::as_array)
        .ok_or("Cost response missing data")?;
    let more = v
        .get("has_more")
        .and_then(Value::as_bool)
        .ok_or("Cost response missing pagination state")?;
    let cursor = v
        .get("next_page")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if more && cursor.is_none() {
        return Err("Cost response missing next cursor".into());
    }
    let mut total = 0.0;
    for bucket in data {
        let results = bucket
            .get("results")
            .and_then(Value::as_array)
            .ok_or("Cost response missing results")?;
        for item in results {
            let amount = item.get("amount").ok_or("Cost item missing amount")?;
            let currency = if anthropic {
                item.get("currency")
            } else {
                amount.get("currency")
            }
            .and_then(Value::as_str)
            .ok_or("Cost item missing currency")?;
            if !currency.eq_ignore_ascii_case("usd") {
                return Err("Unsupported billing currency".into());
            }
            let raw = if anthropic {
                amount
            } else {
                amount.get("value").ok_or("Cost item missing value")?
            };
            let value = raw
                .as_f64()
                .or_else(|| raw.as_str().and_then(|s| s.parse::<f64>().ok()))
                .ok_or("Invalid cost amount")?;
            if !value.is_finite() {
                return Err("Invalid cost amount".into());
            }
            // Negative credits are legitimate billing adjustments.
            total += if anthropic { value / 100.0 } else { value };
            if !total.is_finite() {
                return Err("Cost total out of range".into());
            }
        }
    }
    Ok((total, more, cursor))
}

/// Returns 00:00:00 UTC on the 1st of the current month.
pub fn current_month_start_utc() -> chrono::DateTime<Utc> {
    let now = Utc::now();
    chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .expect("Valid start of month")
        .and_hms_opt(0, 0, 0)
        .expect("Valid time")
        .and_utc()
}

/// Returns 00:00:00 UTC on the 1st of the next month (the API reset timestamp).
pub fn next_month_start_utc() -> chrono::DateTime<Utc> {
    let now = Utc::now();
    let (next_year, next_month) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .expect("Valid start of next month")
        .and_hms_opt(0, 0, 0)
        .expect("Valid time")
        .and_utc()
}

// ============================================================================
// Network Fetchers
// ============================================================================

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "HTTP initialization failed".into())
}
fn fetch_spend(
    client: &reqwest::blocking::Client,
    key: &str,
    anthropic: bool,
) -> Result<f64, String> {
    let month = current_month_start_utc();
    let endpoint = if anthropic {
        "https://api.anthropic.com/v1/organizations/cost_report"
    } else {
        "https://api.openai.com/v1/organization/costs"
    };
    let mut query = vec![
        ("bucket_width", "1d".to_string()),
        ("limit", "31".to_string()),
    ];
    query.push((
        if anthropic {
            "starting_at"
        } else {
            "start_time"
        },
        if anthropic {
            month.to_rfc3339_opts(SecondsFormat::Secs, true)
        } else {
            month.timestamp().to_string()
        },
    ));
    query.push((
        if anthropic { "ending_at" } else { "end_time" },
        if anthropic {
            Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
        } else {
            Utc::now().timestamp().to_string()
        },
    ));
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = HashSet::new();
    let mut cursor = None;
    let mut total = 0.0;
    for _ in 0..MAX_PAGINATION_PAGES {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Cost request timed out")?;
        let mut request = client
            .get(endpoint)
            .query(&query)
            .timeout(remaining.min(Duration::from_secs(15)));
        if let Some(ref page) = cursor {
            request = request.query(&[("page", page)]);
        }
        request = if anthropic {
            request
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
        } else {
            request.bearer_auth(key)
        };
        let response = request.send().map_err(|_| "Provider request failed")?;
        if !response.status().is_success() {
            return Err(format!(
                "Provider returned HTTP {}",
                response.status().as_u16()
            ));
        }
        let mut body = Vec::new();
        response
            .take(5 * 1024 * 1024 + 1)
            .read_to_end(&mut body)
            .map_err(|_| "Provider response read failed")?;
        if body.len() > 5 * 1024 * 1024 {
            return Err("Provider response too large".into());
        }
        let json: Value = serde_json::from_slice(&body).map_err(|_| "Invalid provider response")?;
        let (amount, more, next) = parse_cost_page(&json, anthropic)?;
        total += amount;
        if !total.is_finite() {
            return Err("Cost total out of range".into());
        }
        if !more {
            return Ok(total);
        }
        let next = next.ok_or("Missing pagination cursor")?;
        if !seen.insert(next.clone()) {
            return Err("Repeated pagination cursor".into());
        }
        cursor = Some(next);
    }
    Err("Cost pagination limit exceeded".into())
}

// ============================================================================
// State Cache & Lifecycle Management
// ============================================================================

#[derive(Debug, Default, Clone)]
struct ApiSlot {
    configured: bool,
    generation: u64,
    month: i64,
    last_success_usd: Option<f64>,
    last_error: Option<String>,
    stale: bool,
}
#[derive(Debug, Default)]
struct SharedState {
    claude: Vec<Meter>,
    anthropic: ApiSlot,
    openai: ApiSlot,
}
static CACHE: OnceLock<Arc<Mutex<SharedState>>> = OnceLock::new();
static REFRESH: AtomicBool = AtomicBool::new(true);
fn get_cache() -> &'static Arc<Mutex<SharedState>> {
    CACHE.get_or_init(|| Arc::new(Mutex::new(SharedState::default())))
}
fn invalidate_api_slot(name: &str, configured: bool) {
    if let Ok(mut state) = get_cache().lock() {
        let slot = if name == "anthropic-admin" {
            &mut state.anthropic
        } else {
            &mut state.openai
        };
        *slot = ApiSlot {
            configured,
            generation: slot.generation.wrapping_add(1),
            stale: true,
            ..Default::default()
        };
    }
    REFRESH.store(true, Ordering::Relaxed);
}
fn build_api_meter(id: &str, label: &str, slot: &ApiSlot, month: i64, reset: i64) -> Meter {
    let current = slot.month == month;
    Meter {
        id: id.into(),
        provider: id.into(),
        label: label.into(),
        used_pct: None,
        spent_usd: if current { slot.last_success_usd } else { None },
        resets_at: Some(reset),
        source: "api".into(),
        stale: !current || slot.stale,
        status: Some(
            if slot.last_error.is_some() {
                "error"
            } else if !slot.configured {
                "unconfigured"
            } else if !current || slot.stale || slot.last_success_usd.is_none() {
                "stale"
            } else {
                "ok"
            }
            .into(),
        ),
        error: slot.last_error.clone(),
    }
}
pub fn current_snapshot() -> Vec<Meter> {
    let month = current_month_start_utc().timestamp();
    let reset = next_month_start_utc().timestamp();
    let state = get_cache().lock().unwrap_or_else(|p| p.into_inner());
    let mut meters = state.claude.clone();
    if state.anthropic.configured {
        meters.push(build_api_meter(
            "anthropic-api",
            "Claude API",
            &state.anthropic,
            month,
            reset,
        ));
    }
    if state.openai.configured {
        meters.push(build_api_meter(
            "openai-api",
            "OpenAI API",
            &state.openai,
            month,
            reset,
        ));
    }
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
/// A key that cannot be read may still exist, so the provider stays on the island with an error.
fn record_key_read_failure(slot: &mut ApiSlot, generation: u64) {
    if slot.generation == generation {
        slot.configured = true;
        slot.last_error = Some("Credential Manager read failed".into());
        slot.stale = true;
    }
}
fn commit_result(
    slot: &mut ApiSlot,
    generation: u64,
    month: i64,
    configured: bool,
    result: Result<f64, String>,
) {
    if slot.generation != generation {
        return;
    }
    if slot.month != month {
        slot.last_success_usd = None;
    }
    slot.month = month;
    slot.configured = configured;
    slot.stale = true;
    match result {
        Ok(value) => {
            slot.last_success_usd = Some(value);
            slot.last_error = None;
            slot.stale = false;
        }
        Err(error) => {
            slot.last_error = if configured { Some(error) } else { None };
            if !configured {
                slot.last_success_usd = None;
            }
        }
    }
}
pub fn start<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || {
        let client = http_client();
        let mut last_file = Instant::now() - Duration::from_secs(60);
        let mut last_api = Instant::now() - Duration::from_secs(600);
        let mut last_month = 0;
        let mut emitted = None;
        loop {
            if last_file.elapsed() >= Duration::from_secs(60) {
                let claude = claude_meters();
                get_cache().lock().unwrap_or_else(|p| p.into_inner()).claude = claude;
                last_file = Instant::now();
            }
            let month = current_month_start_utc().timestamp();
            if REFRESH.swap(false, Ordering::Relaxed)
                || last_api.elapsed() >= Duration::from_secs(600)
                || month != last_month
            {
                for (name, anthropic) in [("anthropic-admin", true), ("openai-admin", false)] {
                    let generation = {
                        let state = get_cache().lock().unwrap_or_else(|p| p.into_inner());
                        if anthropic {
                            state.anthropic.generation
                        } else {
                            state.openai.generation
                        }
                    };
                    let key = get_secret(name); // OS I/O and HTTP intentionally outside cache lock.
                    let result = match (&client, &key) {
                        (Ok(client), Ok(Some(key))) => fetch_spend(client, key, anthropic),
                        (Err(_), Ok(Some(_))) => Err("HTTP initialization failed".into()),
                        (_, Err(error)) => Err(error.clone()),
                        _ => Err("Not configured".into()),
                    };
                    let mut state = get_cache().lock().unwrap_or_else(|p| p.into_inner());
                    let slot = if anthropic {
                        &mut state.anthropic
                    } else {
                        &mut state.openai
                    };
                    match key {
                        Ok(key) => commit_result(slot, generation, month, key.is_some(), result),
                        Err(_) => record_key_read_failure(slot, generation),
                    }
                }
                last_api = Instant::now();
                last_month = month;
            }
            let snapshot = current_snapshot();
            if emitted.as_ref() != Some(&snapshot) {
                let _ = app.emit("usage", &snapshot);
                emitted = Some(snapshot);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;
    use std::time::Duration;

    #[test]
    fn test_credential_allowlist() {
        assert!(is_allowed_secret_name("anthropic-admin"));
        assert!(is_allowed_secret_name("openai-admin"));
        assert!(is_allowed_secret_name(" anthropic-admin "));
        assert!(!is_allowed_secret_name("aws-secret"));
        assert!(!is_allowed_secret_name("claude-api"));
        assert!(!is_allowed_secret_name(""));
    }

    #[test]
    fn test_error_sanitization() {
        for raw in [
            "Localized ??? with sk-admin-secret",
            "x-api-key: private",
            "Bearer secret",
            "??????",
        ] {
            let safe = sanitize_error(raw);
            assert!(!safe.contains("secret"));
            assert!(!safe.contains("private"));
        }
    }

    #[test]
    fn test_bom_stripping() {
        let with_bom = [0xEF, 0xBB, 0xBF, b'{', b'}'];
        let stripped = strip_bom(&with_bom);
        assert_eq!(stripped, b"{}");

        let without_bom = b"{}";
        assert_eq!(strip_bom(without_bom), b"{}");
    }

    #[test]
    fn test_parse_claude_status_valid() {
        let json_bytes = include_bytes!("../tests/fixtures/claude_valid.json");
        let now = SystemTime::now();
        let meters = parse_claude_status_content(json_bytes, Some(now));

        assert_eq!(meters.len(), 2);

        let h5 = meters.iter().find(|m| m.id == "claude-5h").unwrap();
        assert_eq!(h5.used_pct, Some(35.0));
        assert_eq!(h5.resets_at, Some(1793629800)); // 2026-11-02T14:30:00Z
        assert!(!h5.stale);
        assert_eq!(h5.status.as_deref(), Some("ok"));

        let week = meters.iter().find(|m| m.id == "claude-week").unwrap();
        assert_eq!(week.used_pct, Some(12.5));
        assert_eq!(week.resets_at, Some(1793620800));
        assert!(!week.stale);
        assert_eq!(week.status.as_deref(), Some("ok"));
    }

    #[test]
    fn test_parse_claude_status_bom() {
        let json_bytes = include_bytes!("../tests/fixtures/claude_bom.json");
        let now = SystemTime::now();
        let meters = parse_claude_status_content(json_bytes, Some(now));

        assert_eq!(meters.len(), 2);
        let h5 = meters.iter().find(|m| m.id == "claude-5h").unwrap();
        assert_eq!(h5.used_pct, Some(68.0));
        let week = meters.iter().find(|m| m.id == "claude-week").unwrap();
        assert_eq!(week.used_pct, Some(45.0));
    }

    #[test]
    fn test_parse_claude_status_null() {
        let json_bytes = include_bytes!("../tests/fixtures/claude_null.json");
        let now = SystemTime::now();
        let meters = parse_claude_status_content(json_bytes, Some(now));

        assert_eq!(meters.len(), 2);
        for m in &meters {
            assert_eq!(m.used_pct, None);
            assert_eq!(m.status.as_deref(), Some("unavailable"));
        }
    }

    #[test]
    fn test_parse_claude_status_malformed() {
        let json_bytes = include_bytes!("../tests/fixtures/claude_malformed.json");
        let now = SystemTime::now();
        let meters = parse_claude_status_content(json_bytes, Some(now));

        assert_eq!(meters.len(), 2);
        for m in &meters {
            assert_eq!(m.used_pct, None);
            assert!(m.stale);
            assert_eq!(m.status.as_deref(), Some("unavailable"));
            assert!(m.error.is_some());
        }
    }

    #[test]
    fn test_parse_claude_status_nonfinite() {
        let json_bytes = include_bytes!("../tests/fixtures/claude_nonfinite.json");
        let now = SystemTime::now();
        let meters = parse_claude_status_content(json_bytes, Some(now));

        assert_eq!(meters.len(), 2);
        let h5 = meters.iter().find(|m| m.id == "claude-5h").unwrap();
        assert_eq!(h5.used_pct, None);
        assert_eq!(h5.resets_at, None);
        assert_eq!(h5.status.as_deref(), Some("unavailable"));
    }

    #[test]
    fn test_parse_claude_status_stale() {
        let json_bytes = include_bytes!("../tests/fixtures/claude_valid.json");
        // Simulate file modified 700 seconds ago (> 600s threshold)
        let old_time = SystemTime::now() - Duration::from_secs(700);
        let meters = parse_claude_status_content(json_bytes, Some(old_time));

        assert_eq!(meters.len(), 2);
        for m in &meters {
            assert!(m.stale);
            assert_eq!(m.status.as_deref(), Some("stale"));
        }
    }

    #[test]
    fn test_anthropic_cost_page_currency_and_pagination() {
        let page1_bytes = include_bytes!("../tests/fixtures/anthropic_page1.json");
        let json1: Value = serde_json::from_slice(page1_bytes).unwrap();
        let (usd1, has_more1, next_page1) = parse_anthropic_cost_page(&json1).unwrap();

        // 1250.50 cents + 749.50 cents = 2000.00 cents = $20.00
        assert!((usd1 - 20.0).abs() < 1e-6);
        assert!(has_more1);
        assert_eq!(next_page1.as_deref(), Some("cursor_page_2"));

        let page2_bytes = include_bytes!("../tests/fixtures/anthropic_page2.json");
        let json2: Value = serde_json::from_slice(page2_bytes).unwrap();
        let (usd2, has_more2, next_page2) = parse_anthropic_cost_page(&json2).unwrap();

        // 550.00 cents = $5.50
        assert!((usd2 - 5.50).abs() < 1e-6);
        assert!(!has_more2);
        assert_eq!(next_page2, None);

        let total_usd = usd1 + usd2;
        assert!((total_usd - 25.50).abs() < 1e-6);
    }

    #[test]
    fn test_openai_costs_page_currency_and_pagination() {
        let page1_bytes = include_bytes!("../tests/fixtures/openai_page1.json");
        let json1: Value = serde_json::from_slice(page1_bytes).unwrap();
        let (usd1, has_more1, next_page1) = parse_openai_costs_page(&json1).unwrap();

        assert!((usd1 - 15.25).abs() < 1e-6);
        assert!(has_more1);
        assert_eq!(next_page1.as_deref(), Some("page_cursor_2"));

        let page2_bytes = include_bytes!("../tests/fixtures/openai_page2.json");
        let json2: Value = serde_json::from_slice(page2_bytes).unwrap();
        let (usd2, has_more2, next_page2) = parse_openai_costs_page(&json2).unwrap();

        assert!((usd2 - 4.75).abs() < 1e-6);
        assert!(!has_more2);
        assert_eq!(next_page2, None);

        let total_usd = usd1 + usd2;
        assert!((total_usd - 20.00).abs() < 1e-6);
    }

    #[test]
    fn test_utc_month_calculations() {
        let start = current_month_start_utc();
        let next = next_month_start_utc();
        assert!(next > start);
        assert_eq!(start.day(), 1);
        assert_eq!(start.hour(), 0);
        assert_eq!(start.minute(), 0);
        assert_eq!(next.day(), 1);
    }

    #[test]
    fn test_dto_serialization_camel_case() {
        let meter = Meter {
            id: "claude-5h".to_string(),
            provider: "claude".to_string(),
            label: "Claude session (5 h)".to_string(),
            used_pct: Some(42.5),
            spent_usd: None,
            resets_at: Some(1700000000),
            source: "official".to_string(),
            stale: false,
            status: Some("ok".to_string()),
            error: None,
        };

        let json_str = serde_json::to_string(&meter).unwrap();
        assert!(json_str.contains("\"usedPct\":42.5"));
        assert!(json_str.contains("\"resetsAt\":1700000000"));
        assert!(!json_str.contains("used_pct"));
        assert!(!json_str.contains("resets_at"));
    }
    #[test]
    fn malformed_costs_are_not_zero() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"data":[{}],"has_more":false}),
            serde_json::json!({"data":[{"results":[{"amount":null,"currency":"USD"}]}],"has_more":false}),
            serde_json::json!({"data":[],"has_more":true}),
        ] {
            assert!(parse_anthropic_cost_page(&value).is_err());
        }
        assert_eq!(
            parse_anthropic_cost_page(&serde_json::json!({"data":[],"has_more":false}))
                .unwrap()
                .0,
            0.0
        );
    }
    #[test]
    fn stale_generation_and_month_cannot_publish_old_spend() {
        let mut slot = ApiSlot {
            generation: 2,
            configured: true,
            month: 10,
            last_success_usd: Some(30.0),
            ..Default::default()
        };
        commit_result(&mut slot, 1, 11, true, Ok(90.0));
        assert_eq!(slot.last_success_usd, Some(30.0));
        assert_eq!(
            build_api_meter("openai-api", "API", &slot, 11, 20).spent_usd,
            None
        );
        commit_result(&mut slot, 2, 11, true, Err("Offline".into()));
        assert_eq!(slot.last_success_usd, None);
    }
    #[test]
    fn rejects_unsupported_currency_and_out_of_range_percentage() {
        assert_eq!(extract_percentage(Some(&serde_json::json!(101))), None);
        assert!(parse_openai_costs_page(&serde_json::json!({"data":[{"results":[{"amount":{"value":12,"currency":"EUR"}}]}],"has_more":false})).is_err());
    }
    #[test]
    fn missing_reset_is_not_a_healthy_limit() {
        let meters = parse_claude_status_content(
            br#"{"rate_limits":{"five_hour":{"used_percentage":42}}}"#,
            Some(SystemTime::now()),
        );
        assert_eq!(meters[0].status.as_deref(), Some("unavailable"));
    }
    #[test]
    fn unconfigured_api_slot_is_marked_and_a_configured_failure_is_not() {
        let unconfigured = build_api_meter("openai-api", "API", &ApiSlot::default(), 10, 20);
        assert_eq!(unconfigured.status.as_deref(), Some("unconfigured"));
        let failed = ApiSlot {
            configured: true,
            last_error: Some("Could not reach the provider".into()),
            ..Default::default()
        };
        let meter = build_api_meter("openai-api", "API", &failed, 10, 20);
        assert_eq!(meter.status.as_deref(), Some("error"));
    }
    /// `current_snapshot` reads the shared cache, so these tests take turns on it.
    static SNAPSHOT_TESTS: Mutex<()> = Mutex::new(());
    fn with_cache<T>(set: impl FnOnce(&mut SharedState), read: impl FnOnce() -> T) -> T {
        let _turn = SNAPSHOT_TESTS.lock().unwrap_or_else(|p| p.into_inner());
        let reset = || {
            *get_cache().lock().unwrap_or_else(|p| p.into_inner()) = SharedState::default();
        };
        reset();
        set(&mut get_cache().lock().unwrap_or_else(|p| p.into_inner()));
        let value = read();
        reset();
        value
    }
    fn snapshot_ids() -> Vec<String> {
        current_snapshot().into_iter().map(|m| m.id).collect()
    }
    #[test]
    fn snapshot_omits_sources_that_are_not_connected() {
        let ids = with_cache(|_| {}, snapshot_ids);
        assert!(ids.is_empty());
        let ids = with_cache(
            |state| {
                state.anthropic.configured = true;
                state.claude = unavailable_claude_meters("Status file exceeds size limit (2MB)");
            },
            snapshot_ids,
        );
        assert_eq!(ids, ["claude-5h", "claude-week", "anthropic-api"]);
    }
    #[test]
    fn first_credential_read_failure_keeps_the_provider_in_the_snapshot() {
        let meters = with_cache(
            |state| record_key_read_failure(&mut state.openai, 0),
            current_snapshot,
        );
        assert_eq!(meters.len(), 1);
        assert_eq!(meters[0].id, "openai-api");
        assert_eq!(meters[0].status.as_deref(), Some("error"));
    }
    #[test]
    fn credential_read_failure_from_an_old_generation_is_ignored() {
        let mut slot = ApiSlot {
            generation: 2,
            ..Default::default()
        };
        record_key_read_failure(&mut slot, 1);
        assert!(!slot.configured);
        assert_eq!(slot.last_error, None);
    }
    #[test]
    fn status_file_older_than_the_weekly_window_is_not_connected() {
        let now = SystemTime::now();
        let aged = |secs: u64| Some(now - Duration::from_secs(secs));
        assert!(status_file_connected(aged(STALE_THRESHOLD_SECS + 1), now));
        assert!(status_file_connected(aged(CONNECTED_THRESHOLD_SECS), now));
        assert!(!status_file_connected(
            aged(CONNECTED_THRESHOLD_SECS + 1),
            now
        ));
        assert!(status_file_connected(None, now));
        assert!(status_file_connected(
            Some(now + Duration::from_secs(60)),
            now
        ));
    }
    #[test]
    fn credential_read_error_preserves_cached_total() {
        let slot = ApiSlot {
            configured: true,
            month: 10,
            last_success_usd: Some(12.0),
            last_error: Some("Credential Manager read failed".into()),
            stale: true,
            ..Default::default()
        };
        let meter = build_api_meter("openai-api", "API", &slot, 10, 20);
        assert_eq!(meter.spent_usd, Some(12.0));
        assert_eq!(meter.status.as_deref(), Some("error"));
    }
}
