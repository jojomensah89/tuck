# Tuck AI Usage Backend Setup & Architecture Guide

This document describes the design, configuration, official API references, and native wiring instructions for the Tuck AI Usage Backend.

---

## 1. Official Documentation & References

### 1.1 Anthropic Organization Cost Report API
* **Documentation**: [Anthropic Admin API - Cost Report](https://docs.anthropic.com/en/api/cost-report)
* **Endpoint**: `GET https://api.anthropic.com/v1/organizations/cost_report`
* **Authentication**: `x-api-key: <anthropic-admin-key>`, `anthropic-version: 2023-06-01`
* **Query Parameters**:
  * `starting_at`: RFC 3339 timestamp (e.g. `2026-11-01T00:00:00Z` for start of current UTC month)
  * `bucket_width`: `1d`
  * `limit`: `31`
  * `page`: Cursor token for subsequent pages
* **Currency & Units**:
  * In the response `data[].results[].amount`, Anthropic reports values in **cents** (minor currency unit) as a decimal string (e.g., `"1250.50"` cents = `$12.5050` USD).
  * Tuck parses this field and divides by `100.0` to calculate accurate USD spending.
* **Pagination**:
  * Response contains `has_more: bool` and `next_page: Option<String>`. Tuck pages through all records until `has_more` is `false` or maximum safety bounds (50 pages) are reached.

### 1.2 OpenAI Organization Costs API
* **Documentation**: [OpenAI API Reference - Organization Costs](https://platform.openai.com/docs/api-reference/organization-costs)
* **Endpoint**: `GET https://api.openai.com/v1/organization/costs`
* **Authentication**: `Authorization: Bearer <openai-admin-key>`
* **Query Parameters**:
  * `start_time`: Unix epoch seconds for start of current UTC month
  * `bucket_width`: `1d`
  * `limit`: `31`
  * `page`: Cursor token for subsequent pages
* **Currency & Units**:
  * In `data[].results[].amount.value`, OpenAI reports values directly in **USD** (`f64`).
* **Pagination**:
  * Response contains `has_more: bool` and `next_page: Option<String>`. Tuck traverses pages bounded by a maximum safety limit.

### 1.3 Claude Code Statusline
* **Documentation**: [Claude Code Statusline Documentation](https://docs.anthropic.com/en/docs/agents-and-tools/claude-code)
* **Integration Pattern**: Claude Code provides a statusline hook that executes a command on each update and sends session telemetry to `stdin` as UTF-8 JSON.
* **Schema**:
  * Rate limits are reported under `/rate_limits/five_hour` and `/rate_limits/seven_day`.
  * Each limit object contains:
    * `used_percentage`: `f64` (scale 0.0 .. 100.0)
    * `resets_at`: RFC 3339 timestamp string or Unix epoch integer seconds
* **Staleness**: If the status file has not been modified within 600 seconds (10 minutes), Tuck flags the data as stale (`stale: true`, `status: "stale"`).

---

## 2. Windows Credential Manager Security

Admin API keys are write-only secrets managed by the Windows Credential Manager:
* **Service Name**: `"tuck"`
* **Allowed Secret Names**:
  * `"anthropic-admin"`
  * `"openai-admin"`
* **Write-Only Security Policy**:
  * Secrets are never logged, serialized to files, or returned over Tauri IPC commands.
  * Setting an empty string or whitespace deletes the stored credential from the Windows Credential Manager.
  * Any secret name outside the allowlist is rejected with a generic validation error without echoing the input.
* **Error Sanitization**:
  * External keyring and request errors use fixed generic messages. Raw external details are withheld to avoid exposing credentials or personal data, including localized errors.

---

## 3. Data Transfer Objects (DTO)

Meter objects transferred over Tauri IPC to the frontend follow camelCase naming:

```typescript
export interface Meter {
  id: string;             // "claude-5h" | "claude-week" | "anthropic-api" | "openai-api"
  provider: string;       // "claude" | "anthropic-api" | "openai-api"
  label: string;          // Human-readable display label
  usedPct?: number;       // Percentage (0..100) for quota-based tiers
  spentUsd?: number;      // Month-to-date spending in USD for pay-per-token APIs
  resetsAt?: number;      // Unix timestamp (seconds)
  source: string;         // "official" | "api"
  stale: bool;            // true if >600s old or refresh failed
  status?: string;        // "ok" | "stale" | "unavailable" | "error" | "unconfigured"
  error?: string;         // Redacted error message if applicable
}
```

### No Fabrication Invariant
* A source that is not connected (no status file or one older than the 7-day window, no saved key, an installed client that is missing or signed out) returns no meter, so it does not appear on the island.
* A connected source that fails returns `used_pct: None` and `spent_usd: None` with `status: "unavailable"` or `status: "error"`, or its last real values marked stale.
* Tuck never returns `0.0` or `0%` for unmeasured or missing data.

### Frontend Ownership
* **Budgets**: User budget thresholds and alerts are managed entirely on the frontend.
* **Subscriptions & Estimates**: Subscription costs are separate from API spending. ChatGPT counts are user-configured manual estimates.

---

## 4. Claude Code Statusline Setup Instructions

Tuck ships a PowerShell statusline forwarder at `scripts/tuck-statusline.ps1`.

### Setup Steps:
1. Ensure the target directory `%APPDATA%\Tuck` exists (the script creates it automatically if missing).
2. Configure Claude Code to use the Tuck statusline script by updating `~/.claude/settings.json`:
   ```json
   {
     "statusLine": {
       "type": "command",
       "command": "powershell.exe -NoProfile -ExecutionPolicy Bypass -File \"C:\\path\\to\\tuck\\scripts\\tuck-statusline.ps1\""
     }
   }
   ```
3. Whenever Claude Code runs a session, it pipes its JSON payload to the script, which atomically saves it to `%APPDATA%\Tuck\claude-status.json` and outputs `Tuck`.
4. Tuck also checks the legacy guide fallback path `%APPDATA%\Notch\claude-status.json` if the Tuck directory is not yet present.

---

## 5. Coordinator & Native Worker Integration Guide

To wire `src-tauri/src/usage.rs` into the main application:

### Step 1: Update `src-tauri/Cargo.toml`
Add the following dependencies under `[dependencies]`:
```toml
chrono = { version = "0.4", features = ["serde"] }
keyring = { version = "3", features = ["windows-native"] }
reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }
```

### Step 2: Update `src-tauri/src/lib.rs`
1. Declare the module:
   ```rust
   pub mod usage;
   ```
2. Register commands in `tauri::Builder`:
   ```rust
   .invoke_handler(tauri::generate_handler![
       // existing commands...
       usage::usage_now,
       usage::set_secret,
   ])
   ```
3. Start the background service in the Tauri setup hook:
   ```rust
   .setup(|app| {
       // start usage polling service
       usage::start(app.handle().clone());
       Ok(())
   })
   ```

## Review corrections and validation

After independent review, the cache snapshot became memory-only. Keyring and HTTP calls execute outside its mutex; credential edits clear cached spending and increment a generation so in-flight old-key requests cannot overwrite the new state. UTC month changes hide old totals. Request query parameters are encoded, redirects disabled, each provider fetch limited to 30 seconds total, each response capped at 5 MB, and malformed/missing amounts, currencies or pagination fail explicitly. Valid empty results can report zero; malformed results cannot.

Raw external error details are withheld rather than passed through a best-effort token sanitizer. The statusline forwarder uses an atomic unique temporary-file replacement and leaves the existing file intact on a failed replacement.

Coordinator validation: 16 usage unit tests plus 10 native tests pass against the wired Windows application. Formatting and Clippy pass. No real provider credentials or organization billing responses were used for validation.
