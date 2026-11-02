//! Tuck AI Usage Backend
//!
//! Provides background polling and non-blocking IPC snapshots for AI usage meters:
//! - Claude Pro / Max session & weekly rate limits (fed by Claude Code statusline script)
//! - Anthropic Admin API month-to-date spend (`cost_report` endpoint)
//! - OpenAI Organization Costs API month-to-date spend (`organization/costs` endpoint)
//!
//! Credentials are stored exclusively in the Windows Credential Manager under the `tuck` service.

use chrono::{Datelike, SecondsFormat, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime},
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
    let mut words: Vec<String> = Vec::new();

    for line in input.lines() {
        let mut line_out = String::new();
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // Check for Bearer token
            if line[i..].starts_with("Bearer ") {
                line_out.push_str("Bearer [REDACTED]");
                i += 7;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'"' && bytes[i] != b'\'' {
                    i += 1;
                }
                continue;
            }

            // Check for x-api-key header
            let remaining = &line[i..];
            if remaining.len() >= 10 && remaining[..10].eq_ignore_ascii_case("x-api-key:") {
                line_out.push_str("x-api-key: [REDACTED]");
                i += 10;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'"' && bytes[i] != b'\'' && bytes[i] != b',' {
                    i += 1;
                }
                continue;
            }

            // Check for sk-ant-admin
            if remaining.starts_with("sk-ant-admin") {
                line_out.push_str("sk-ant-admin[REDACTED]");
                i += 12;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_') {
                    i += 1;
                }
                continue;
            }

            // Check for sk-admin-
            if remaining.starts_with("sk-admin-") {
                line_out.push_str("sk-admin-[REDACTED]");
                i += 9;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_') {
                    i += 1;
                }
                continue;
            }

            // Check for generic sk-
            if remaining.starts_with("sk-") {
                let start = i;
                let mut end = i + 3;
                while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-' || bytes[end] == b'_') {
                    end += 1;
                }
                if end - start >= 12 {
                    line_out.push_str("sk-[REDACTED]");
                    i = end;
                    continue;
                }
            }

            // Safe UTF-8 character consumption
            if let Some(ch) = remaining.chars().next() {
                line_out.push(ch);
                i += ch.len_utf8();
            } else {
                break;
            }
        }
        words.push(line_out);
    }

    if input.is_empty() {
        String::new()
    } else {
        words.join("\n")
    }
}

// ============================================================================
// Windows Credential Manager (Keyring) Integration
// ============================================================================

/// Reads a write-only secret from Windows Credential Manager.
/// Secrets are never logged, serialized to disk, or returned to the client.
pub fn get_secret(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if !is_allowed_secret_name(trimmed) {
        return None;
    }
    let entry = keyring::Entry::new(KEYRING_SERVICE, trimmed).ok()?;
    match entry.get_password() {
        Ok(pw) => {
            let secret = pw.trim().to_string();
            if secret.is_empty() {
                None
            } else {
                Some(secret)
            }
        }
        Err(_) => None,
    }
}

/// Stores or deletes an administrative secret in Windows Credential Manager.
///
/// An empty or whitespace-only value explicitly deletes the stored credential.
/// Frontend can pass empty string to delete or non-empty string to update.
#[tauri::command]
pub fn set_secret(name: String, value: String) -> Result<(), String> {
    let name_trimmed = name.trim();
    if !is_allowed_secret_name(name_trimmed) {
        return Err(format!(
            "Secret '{}' is not permitted. Only 'anthropic-admin' and 'openai-admin' are allowed.",
            name_trimmed
        ));
    }

    let entry = keyring::Entry::new(KEYRING_SERVICE, name_trimmed)
        .map_err(|e| format!("Keyring error: {}", sanitize_error(&e.to_string())))?;

    let val_trimmed = value.trim();
    if val_trimmed.is_empty() {
        // Explicit deletion
        match entry.delete_credential() {
            Ok(()) => {
                invalidate_api_slot(name_trimmed);
                Ok(())
            }
            Err(keyring::Error::NoEntry) => {
                invalidate_api_slot(name_trimmed);
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
        invalidate_api_slot(name_trimmed);
        Ok(())
    }
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
    if num.is_finite() && !num.is_nan() {
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

                    let status = if used_pct.is_none() {
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
            let err_msg = format!("Malformed JSON in status file: {}", err);
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
        let tuck_path = PathBuf::from(&appdata).join("Tuck").join("claude-status.json");
        if tuck_path.exists() {
            return (Some(tuck_path), true);
        }
        let notch_path = PathBuf::from(&appdata).join("Notch").join("claude-status.json");
        if notch_path.exists() {
            return (Some(notch_path), true);
        }
        return (Some(tuck_path), false);
    }
    (None, false)
}

/// Reads and parses Claude Code statusline meters from disk.
pub fn claude_meters() -> Vec<Meter> {
    let (target_path, exists) = resolve_claude_status_path();
    let Some(path) = target_path else {
        return unavailable_claude_meters("APPDATA environment variable not set");
    };

    if !exists {
        return unavailable_claude_meters(
            "Status file not found in %APPDATA%\\Tuck or %APPDATA%\\Notch",
        );
    }

    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return unavailable_claude_meters(&format!("Unable to read metadata: {}", e)),
    };

    if meta.len() > MAX_STATUS_FILE_BYTES {
        return unavailable_claude_meters("Status file exceeds size limit (2MB)");
    }

    let modified = meta.modified().ok();
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => return unavailable_claude_meters(&format!("Unable to read status file: {}", e)),
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
pub fn parse_anthropic_cost_page(json: &Value) -> Result<(f64, bool, Option<String>), String> {
    let data = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Missing or invalid 'data' array in Anthropic response".to_string())?;

    let mut page_cents = 0.0;
    for bucket in data {
        if let Some(results) = bucket.get("results").and_then(Value::as_array) {
            for item in results {
                let amount_cents = if let Some(s) = item.get("amount").and_then(Value::as_str) {
                    s.parse::<f64>()
                        .map_err(|e| format!("Invalid amount decimal: {}", e))?
                } else if let Some(n) = item.get("amount").and_then(Value::as_f64) {
                    n
                } else {
                    continue;
                };

                if !amount_cents.is_finite() || amount_cents.is_nan() {
                    return Err("Non-finite amount encountered in Anthropic response".to_string());
                }

                page_cents += amount_cents;
            }
        }
    }

    let has_more = json
        .get("has_more")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let next_page = json
        .get("next_page")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());

    let page_usd = page_cents / 100.0;
    Ok((page_usd, has_more, next_page))
}

/// Parses a single page from the OpenAI Organization Costs endpoint.
///
/// In OpenAI's official `organization/costs` API, `amount.value` is reported in **USD dollars**.
///
/// Returns `(page_usd, has_more, next_page_cursor)`.
pub fn parse_openai_costs_page(json: &Value) -> Result<(f64, bool, Option<String>), String> {
    let data = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Missing or invalid 'data' array in OpenAI response".to_string())?;

    let mut page_usd = 0.0;
    for bucket in data {
        if let Some(results) = bucket.get("results").and_then(Value::as_array) {
            for item in results {
                let amount_obj = item.get("amount");
                let val = if let Some(n) =
                    amount_obj.and_then(|a| a.get("value")).and_then(Value::as_f64)
                {
                    n
                } else if let Some(s) =
                    amount_obj.and_then(|a| a.get("value")).and_then(Value::as_str)
                {
                    s.parse::<f64>()
                        .map_err(|e| format!("Invalid amount value string: {}", e))?
                } else {
                    continue;
                };

                if !val.is_finite() || val.is_nan() {
                    return Err("Non-finite amount encountered in OpenAI response".to_string());
                }

                page_usd += val;
            }
        }
    }

    let has_more = json
        .get("has_more")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let next_page = json
        .get("next_page")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());

    Ok((page_usd, has_more, next_page))
}

// ============================================================================
// UTC Month Calculations
// ============================================================================

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

fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

/// Fetches month-to-date spend from Anthropic Admin API with pagination.
fn fetch_anthropic_spend(client: &reqwest::blocking::Client, key: &str) -> Result<f64, String> {
    let starting_at = current_month_start_utc().to_rfc3339_opts(SecondsFormat::Secs, true);
    let mut total_usd = 0.0;
    let mut page_cursor: Option<String> = None;
    let mut page_count = 0;

    loop {
        page_count += 1;
        if page_count > MAX_PAGINATION_PAGES {
            return Err("Pagination limit reached for Anthropic cost report".to_string());
        }

        let mut url = format!(
            "https://api.anthropic.com/v1/organizations/cost_report?starting_at={}&bucket_width=1d&limit=31",
            starting_at
        );
        if let Some(cursor) = &page_cursor {
            url.push_str("&page=");
            url.push_str(cursor);
        }

        let resp = client
            .get(&url)
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .map_err(|e| sanitize_error(&e.to_string()))?;

        if !resp.status().is_success() {
            return Err(format!("Anthropic API returned status {}", resp.status()));
        }

        let val: Value = resp.json().map_err(|e| sanitize_error(&e.to_string()))?;
        let (page_usd, has_more, next_cursor) = parse_anthropic_cost_page(&val)?;
        total_usd += page_usd;

        if has_more && next_cursor.is_some() {
            page_cursor = next_cursor;
        } else {
            break;
        }
    }

    Ok(total_usd)
}

/// Fetches month-to-date spend from OpenAI Organization Costs API with pagination.
fn fetch_openai_spend(client: &reqwest::blocking::Client, key: &str) -> Result<f64, String> {
    let start_time = current_month_start_utc().timestamp();
    let mut total_usd = 0.0;
    let mut page_cursor: Option<String> = None;
    let mut page_count = 0;

    loop {
        page_count += 1;
        if page_count > MAX_PAGINATION_PAGES {
            return Err("Pagination limit reached for OpenAI organization costs".to_string());
        }

        let mut url = format!(
            "https://api.openai.com/v1/organization/costs?start_time={}&bucket_width=1d&limit=31",
            start_time
        );
        if let Some(cursor) = &page_cursor {
            url.push_str("&page=");
            url.push_str(cursor);
        }

        let resp = client
            .get(&url)
            .bearer_auth(key)
            .send()
            .map_err(|e| sanitize_error(&e.to_string()))?;

        if !resp.status().is_success() {
            return Err(format!("OpenAI API returned status {}", resp.status()));
        }

        let val: Value = resp.json().map_err(|e| sanitize_error(&e.to_string()))?;
        let (page_usd, has_more, next_cursor) = parse_openai_costs_page(&val)?;
        total_usd += page_usd;

        if has_more && next_cursor.is_some() {
            page_cursor = next_cursor;
        } else {
            break;
        }
    }

    Ok(total_usd)
}

// ============================================================================
// State Cache & Lifecycle Management
// ============================================================================

#[derive(Debug, Default, Clone)]
struct ApiSlot {
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

fn get_cache() -> &'static Arc<Mutex<SharedState>> {
    CACHE.get_or_init(|| Arc::new(Mutex::new(SharedState::default())))
}

fn invalidate_api_slot(name: &str) {
    if let Ok(mut state) = get_cache().lock() {
        if name == "anthropic-admin" {
            state.anthropic.last_error = None;
            state.anthropic.stale = true;
        } else if name == "openai-admin" {
            state.openai.last_error = None;
            state.openai.stale = true;
        }
    }
}

fn build_api_meter(
    id: &str,
    provider: &str,
    label: &str,
    secret_key_name: &str,
    slot: &ApiSlot,
    resets_at: i64,
) -> Meter {
    let has_key = get_secret(secret_key_name).is_some();
    if !has_key {
        return Meter {
            id: id.to_string(),
            provider: provider.to_string(),
            label: label.to_string(),
            used_pct: None,
            spent_usd: None,
            resets_at: Some(resets_at),
            source: "api".to_string(),
            stale: true,
            status: Some("unconfigured".to_string()),
            error: None,
        };
    }

    let status = if slot.last_error.is_some() {
        Some("error".to_string())
    } else if slot.stale {
        Some("stale".to_string())
    } else {
        Some("ok".to_string())
    };

    Meter {
        id: id.to_string(),
        provider: provider.to_string(),
        label: label.to_string(),
        used_pct: None,
        spent_usd: slot.last_success_usd,
        resets_at: Some(resets_at),
        source: "api".to_string(),
        stale: slot.stale,
        status,
        error: slot.last_error.clone(),
    }
}

/// Generates a non-blocking cached snapshot of all usage meters.
pub fn current_snapshot() -> Vec<Meter> {
    let reset_timestamp = next_month_start_utc().timestamp();
    let state = get_cache().lock().unwrap();

    let mut meters = if state.claude.is_empty() {
        claude_meters()
    } else {
        state.claude.clone()
    };

    meters.push(build_api_meter(
        "anthropic-api",
        "anthropic-api",
        "Claude API",
        "anthropic-admin",
        &state.anthropic,
        reset_timestamp,
    ));

    meters.push(build_api_meter(
        "openai-api",
        "openai-api",
        "OpenAI API",
        "openai-admin",
        &state.openai,
        reset_timestamp,
    ));

    meters
}

/// Public Tauri command returning a fast, non-blocking cached snapshot of AI meters.
#[tauri::command]
pub async fn usage_now() -> Vec<Meter> {
    current_snapshot()
}

/// Starts the background usage polling service.
///
/// Spawns a background thread that periodically refreshes:
/// - Local Claude statusline file (every 60 seconds)
/// - Configured remote APIs (every 10 minutes)
///
/// Emits `"usage"` events to Tauri listeners on every tick.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || {
        let client = http_client();
        let mut tick: u32 = 0;

        loop {
            let claude = claude_meters();
            let check_apis = tick % 10 == 0;

            {
                let mut state = get_cache().lock().unwrap();
                state.claude = claude;

                if check_apis {
                    // Anthropic
                    if let Some(key) = get_secret("anthropic-admin") {
                        match fetch_anthropic_spend(&client, &key) {
                            Ok(usd) => {
                                state.anthropic.last_success_usd = Some(usd);
                                state.anthropic.last_error = None;
                                state.anthropic.stale = false;
                            }
                            Err(e) => {
                                state.anthropic.last_error = Some(e);
                                state.anthropic.stale = true;
                            }
                        }
                    } else {
                        state.anthropic = ApiSlot {
                            last_success_usd: None,
                            last_error: None,
                            stale: true,
                        };
                    }

                    // OpenAI
                    if let Some(key) = get_secret("openai-admin") {
                        match fetch_openai_spend(&client, &key) {
                            Ok(usd) => {
                                state.openai.last_success_usd = Some(usd);
                                state.openai.last_error = None;
                                state.openai.stale = false;
                            }
                            Err(e) => {
                                state.openai.last_error = Some(e);
                                state.openai.stale = true;
                            }
                        }
                    } else {
                        state.openai = ApiSlot {
                            last_success_usd: None,
                            last_error: None,
                            stale: true,
                        };
                    }
                }
            }

            let snapshot = current_snapshot();
            let _ = app.emit("usage", snapshot);

            tick = tick.wrapping_add(1);
            std::thread::sleep(Duration::from_secs(60));
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
        let raw = "Failed to connect with sk-ant-admin0192837465abc: timeout";
        let sanitized = sanitize_error(raw);
        assert!(!sanitized.contains("sk-ant-admin0192837465abc"));
        assert!(sanitized.contains("sk-ant-admin[REDACTED]"));

        let raw_openai = "Error with key sk-admin-0987654321xyz in request";
        let sanitized_openai = sanitize_error(raw_openai);
        assert!(!sanitized_openai.contains("sk-admin-0987654321xyz"));
        assert!(sanitized_openai.contains("sk-admin-[REDACTED]"));

        let raw_bearer = "Authorization: Bearer secret_token_value_12345";
        let sanitized_bearer = sanitize_error(raw_bearer);
        assert!(!sanitized_bearer.contains("secret_token_value_12345"));
        assert!(sanitized_bearer.contains("Bearer [REDACTED]"));
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
}
