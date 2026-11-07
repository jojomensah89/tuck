//! Read-only quota probes through installed client control protocols.
//! No prompts, turns, login requests, credential reads or tool approvals.
use crate::usage::Meter;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Condvar, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
static REQUESTED: AtomicU64 = AtomicU64::new(0);
static COMPLETED: AtomicU64 = AtomicU64::new(0);
static EPOCH: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static REFRESH: Mutex<bool> = Mutex::new(true);
static WAKE: Condvar = Condvar::new();
fn request_refresh() {
    *REFRESH.lock().unwrap_or_else(|p| p.into_inner()) = true;
    WAKE.notify_all();
}
static METERS: Mutex<Vec<Meter>> = Mutex::new(Vec::new());
static SETTINGS_PATH: OnceLock<PathBuf> = OnceLock::new();
pub fn snapshot() -> Vec<Meter> {
    if !ENABLED.load(Ordering::Relaxed) {
        return Vec::new();
    }
    METERS.lock().unwrap_or_else(|p| p.into_inner()).clone()
}
#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Copy)]
#[serde(default)]
pub struct Preferences {
    pub claude: bool,
    pub codex: bool,
}
static PREFERENCES: Mutex<Preferences> = Mutex::new(Preferences {
    claude: false,
    codex: false,
});
pub fn provider_enabled(name: &str) -> bool {
    let p = PREFERENCES.lock().unwrap_or_else(|p| p.into_inner());
    match name {
        "claude" => p.claude,
        "codex" => p.codex,
        _ => false,
    }
}
fn migrate_preferences(bytes: &[u8]) -> Preferences {
    if bytes == b"true" {
        Preferences {
            claude: true,
            codex: true,
        }
    } else {
        serde_json::from_slice(bytes).unwrap_or_default()
    }
}
#[derive(serde::Serialize)]
pub struct ProviderSetting {
    provider: String,
    installed: bool,
    enabled: bool,
}
#[tauri::command]
pub fn installed_usage_providers() -> Vec<ProviderSetting> {
    ["claude", "codex"]
        .into_iter()
        .map(|name| ProviderSetting {
            provider: name.into(),
            installed: executable(name).is_ok(),
            enabled: provider_enabled(name),
        })
        .collect()
}
#[tauri::command]
pub fn set_usage_provider(
    window: WebviewWindow,
    provider: String,
    enabled: bool,
) -> Result<(), String> {
    if window.label() != "settings" {
        return Err("Change usage sources in settings".into());
    }
    if !["claude", "codex"].contains(&provider.as_str()) {
        return Err("Unsupported provider".into());
    }
    if enabled && executable(&provider).is_err() {
        return Err("App not found".into());
    }
    let path = SETTINGS_PATH.get().ok_or("Usage settings unavailable")?;
    let mut prefs = PREFERENCES.lock().unwrap_or_else(|p| p.into_inner());
    let mut next = *prefs;
    if provider == "claude" {
        next.claude = enabled;
    } else {
        next.codex = enabled;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "Could not save usage settings")?;
    }
    std::fs::write(
        path,
        serde_json::to_vec(&next).map_err(|_| "Could not save usage settings")?,
    )
    .map_err(|_| "Could not save usage settings")?;
    *prefs = next;
    EPOCH.fetch_add(1, Ordering::SeqCst);
    ENABLED.store(next.claude || next.codex, Ordering::SeqCst);
    METERS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|m| m.provider != provider || enabled);
    drop(prefs);
    request_refresh();
    let _ = window
        .app_handle()
        .emit("usage", crate::usage::current_snapshot());
    Ok(())
}
#[tauri::command]
pub fn refresh_installed_usage() {
    request_refresh();
}
pub fn request_manual_refresh() -> u64 {
    let ticket = REQUESTED.fetch_add(1, Ordering::SeqCst) + 1;
    request_refresh();
    ticket
}
pub fn manual_refresh_finished(ticket: u64) -> bool {
    !ENABLED.load(Ordering::SeqCst) || COMPLETED.load(Ordering::SeqCst) >= ticket
}
/// Why a probe returned no quota. Only `NotConnected` keeps a provider off the island:
/// the client is missing, signed out, or on an account without subscription limits.
#[derive(Debug)]
enum ProbeError {
    NotConnected,
    Failed(String),
}
impl From<String> for ProbeError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}
/// The reading a connected provider keeps after a failed probe: its last real values
/// marked stale, or an empty error meter if it has not reported anything yet.
fn after_failed_probe(provider: &str, previous: &[Meter], error: &str) -> Vec<Meter> {
    let mut kept: Vec<Meter> = previous
        .iter()
        .filter(|meter| meter.provider == provider && meter.used_pct.is_some())
        .cloned()
        .collect();
    for meter in &mut kept {
        meter.stale = true;
        meter.status = Some("error".into());
        meter.error = Some(error.into());
    }
    if kept.is_empty() {
        kept.push(Meter {
            id: format!("{provider}-installed"),
            provider: provider.into(),
            label: if provider == "codex" {
                "ChatGPT / Codex"
            } else {
                "Claude Code"
            }
            .into(),
            used_pct: None,
            spent_usd: None,
            resets_at: None,
            source: "official".into(),
            stale: true,
            status: Some("error".into()),
            error: Some(error.into()),
        });
    }
    kept
}
fn quota(
    id: String,
    provider: &str,
    label: String,
    percent: &Value,
    reset: &Value,
) -> Option<Meter> {
    let used = percent
        .as_f64()
        .filter(|p| p.is_finite() && (0.0..=100.0).contains(p))?;
    let reset = crate::usage::parse_unix_timestamp(reset).filter(|n| *n > 0)?;
    Some(Meter {
        id,
        provider: provider.into(),
        label,
        used_pct: Some(used),
        spent_usd: None,
        resets_at: Some(reset),
        source: "official".into(),
        stale: false,
        status: Some("ok".into()),
        error: None,
    })
}
pub fn parse_codex(account: &Value, limits: &Value) -> Result<Vec<Meter>, String> {
    if account["account"]["type"] != "chatgpt" {
        return Err("Sign in to the installed Codex client with ChatGPT. API-key accounts do not expose subscription limits.".into());
    }
    let buckets: Vec<(String, &Value)> = if let Some(map) = limits["rateLimitsByLimitId"]
        .as_object()
        .filter(|m| !m.is_empty())
    {
        map.iter()
            .take(16)
            .map(|(id, value)| (id.clone(), value))
            .collect()
    } else {
        vec![("codex".into(), &limits["rateLimits"])]
    };
    let mut meters = Vec::new();
    for (id, bucket) in buckets {
        let name = bucket["limitName"].as_str().unwrap_or("Codex");
        for key in ["primary", "secondary"] {
            let window = &bucket[key];
            let duration = match window["windowDurationMins"].as_u64() {
                Some(10080) => "weekly".into(),
                Some(n) if n % 60 == 0 => format!("{} h", n / 60),
                Some(n) => format!("{n} min"),
                None => key.into(),
            };
            if let Some(meter) = quota(
                format!("codex-installed-{id}-{key}"),
                "codex",
                format!("{name} {duration}"),
                &window["usedPercent"],
                &window["resetsAt"],
            ) {
                meters.push(meter);
            }
        }
    }
    if meters.is_empty() {
        Err("Installed Codex did not report quota windows".into())
    } else {
        Ok(meters)
    }
}
pub fn parse_claude(value: &Value) -> Result<Vec<Meter>, String> {
    if value["rate_limits_available"] == false {
        return Err("This Claude Code connection does not expose subscription usage".into());
    }
    let limits = value.get("rate_limits").unwrap_or(value);
    let mut meters = Vec::new();
    for (key, label) in [
        ("five_hour", "Claude 5 h"),
        ("seven_day", "Claude weekly"),
        ("seven_day_opus", "Claude weekly Opus"),
        ("seven_day_sonnet", "Claude weekly Sonnet"),
    ] {
        let window = &limits[key];
        let percent = window
            .get("utilization")
            .or_else(|| window.get("used_percentage"))
            .unwrap_or(&Value::Null);
        if let Some(meter) = quota(
            format!("claude-installed-{key}"),
            "claude",
            label.into(),
            percent,
            &window["resets_at"],
        ) {
            meters.push(meter);
        }
    }
    if let Some(scoped) = limits["model_scoped"].as_array() {
        for (index, window) in scoped.iter().take(16).enumerate() {
            let name = window["display_name"].as_str().unwrap_or("model");
            if let Some(meter) = quota(
                format!("claude-installed-scoped-{index}"),
                "claude",
                format!("Claude weekly {name}"),
                &window["utilization"],
                &window["resets_at"],
            ) {
                meters.push(meter);
            }
        }
    }
    if meters.is_empty() {
        Err("Installed Claude Code did not report quota windows".into())
    } else {
        Ok(meters)
    }
}
struct Client {
    child: Child,
    stdin: ChildStdin,
    messages: mpsc::Receiver<Result<Value, String>>,
    reader: Option<std::thread::JoinHandle<()>>,
    deadline: Instant,
    epoch: u64,
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
fn executable(name: &str) -> Result<PathBuf, String> {
    let mut paths = Vec::new();
    if name == "codex" {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            paths.push(PathBuf::from(local).join("Programs/OpenAI/Codex/bin/codex.exe"));
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        paths.push(PathBuf::from(home).join(format!(".local/bin/{name}.exe")));
    }
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(
            std::env::split_paths(&path)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join(format!("{name}.exe"))),
        );
    }
    detected_executable(paths)
        .ok_or_else(|| format!("Install {name} CLI and sign in before enabling its usage meter"))
}
fn detected_executable(paths: Vec<PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find(|path| path.is_file())
}
impl Client {
    fn start(name: &str) -> Result<Self, String> {
        let epoch = EPOCH.load(Ordering::SeqCst);
        let path = executable(name)?;
        let mut command = Command::new(path);
        if name == "codex" {
            command.arg("app-server");
        } else {
            command
                .args([
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--input-format",
                    "stream-json",
                    "--no-session-persistence",
                    "--tools",
                    "",
                    "--mcp-config",
                    "{\"mcpServers\":{}}",
                    "--strict-mcp-config",
                    "--setting-sources",
                    "user",
                    "--settings",
                    "{\"disableAllHooks\":true,\"disableClaudeAiConnectors\":true}",
                ])
                .env("CLAUDE_CODE_ENTRYPOINT", "sdk-ts")
                .env("ENABLE_CLAUDEAI_MCP_SERVERS", "false")
                .env("CLAUDE_CODE_AUTO_CONNECT_IDE", "0")
                .env("CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL", "1");
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            command.current_dir(home);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|_| format!("Could not start installed {name} CLI"))?;
        let stdin = child.stdin.take().ok_or("Client input unavailable")?;
        let stdout = child.stdout.take().ok_or("Client output unavailable")?;
        let (tx, messages) = mpsc::sync_channel(32);
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut total = 0usize;
            for _ in 0..256 {
                let mut bytes = Vec::new();
                let result = Read::take(&mut reader, 1_048_577).read_until(b'\n', &mut bytes);
                total += bytes.len();
                if result.is_err()
                    || bytes.is_empty()
                    || bytes.len() > 1_048_576
                    || total > 8 * 1024 * 1024
                {
                    let _ = tx.try_send(Err("Client output closed or exceeded limits".into()));
                    return;
                }
                if bytes.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let value = serde_json::from_slice::<Value>(&bytes)
                    .map_err(|_| "Client returned an invalid protocol frame".into());
                if tx.try_send(value).is_err() {
                    return;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            messages,
            reader: Some(reader),
            deadline: Instant::now() + Duration::from_secs(25),
            epoch,
        })
    }
    fn send(&mut self, value: &Value) -> Result<(), String> {
        if self.epoch != EPOCH.load(Ordering::SeqCst) {
            return Err("Usage request cancelled".into());
        }
        let mut bytes = serde_json::to_vec(value).map_err(|_| "Invalid control request")?;
        bytes.push(b'\n');
        self.stdin
            .write_all(&bytes)
            .map_err(|_| "Client closed its control input".into())
    }
    fn request(
        &mut self,
        id: u32,
        method: &str,
        params: Value,
        claude: bool,
    ) -> Result<Value, String> {
        let request_id = format!("tuck-{id}");
        if claude {
            self.send(&json!({"type":"control_request","request_id":request_id,"request":params}))?;
        } else {
            self.send(&json!({"id":id,"method":method,"params":params}))?;
        }
        let request_deadline = (Instant::now() + Duration::from_secs(12)).min(self.deadline);
        loop {
            let remaining = request_deadline
                .checked_duration_since(Instant::now())
                .ok_or("Installed client usage request timed out")?;
            if self.epoch != EPOCH.load(Ordering::SeqCst) {
                return Err("Usage request cancelled".into());
            }
            let value = match self
                .messages
                .recv_timeout(remaining.min(Duration::from_millis(50)))
            {
                Ok(value) => value?,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err("Client output closed".into()),
            };
            if claude {
                if value["type"] == "control_response"
                    && value["response"]["request_id"] == request_id
                {
                    if value["response"]["subtype"] != "success" {
                        return Err("Claude Code rejected the read-only usage request. Update/sign in to the CLI.".into());
                    }
                    return Ok(value["response"]["response"].clone());
                }
                if value["type"] == "control_request" {
                    let response = json!({"type":"control_response","response":{"subtype":"error","request_id":value["request_id"],"error":"Read-only usage client does not execute tools or approve requests"}});
                    self.send(&response)?;
                }
            } else {
                if value["id"].as_u64() == Some(id as u64) && value.get("method").is_none() {
                    if value.get("error").is_some() {
                        return Err("Codex rejected the read-only account usage request".into());
                    }
                    return value
                        .get("result")
                        .cloned()
                        .ok_or("Codex response missing result".into());
                }
                if value.get("method").is_some() && value.get("id").is_some() {
                    self.send(&json!({"id":value["id"],"error":{"code":-32601,"message":"Read-only usage client does not handle server requests"}}))?;
                }
            }
        }
    }
}
fn codex_account(account: &Value) -> Result<(), ProbeError> {
    if account["account"]["type"] == "chatgpt" {
        Ok(())
    } else {
        Err(ProbeError::NotConnected)
    }
}
fn claude_usage(value: &Value) -> Result<Vec<Meter>, ProbeError> {
    if value["rate_limits_available"] == false {
        return Err(ProbeError::NotConnected);
    }
    Ok(parse_claude(value)?)
}
fn probe(name: &str) -> Result<Vec<Meter>, ProbeError> {
    executable(name).map_err(|_| ProbeError::NotConnected)?;
    let mut client = Client::start(name)?;
    if name == "codex" {
        client.request(1,"initialize",json!({"clientInfo":{"name":"tuck","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true,"requestAttestation":false}}),false)?;
        client.send(&json!({"method":"initialized","params":{}}))?;
        let account = client.request(2, "account/read", json!({"refreshToken":false}), false)?;
        codex_account(&account)?;
        let limits = client.request(
            3,
            "account/rateLimits/read",
            json!({"excludeResetCreditDetails":true}),
            false,
        )?;
        Ok(parse_codex(&account, &limits)?)
    } else {
        client.request(1, "", json!({"subtype":"initialize"}), true)?;
        let value = client.request(
            2,
            "",
            json!({"subtype":"get_usage","skip_behaviors":true}),
            true,
        )?;
        claude_usage(&value)
    }
}
pub fn start(app: AppHandle) {
    if let Ok(dir) = app.path().app_data_dir() {
        let path = dir.join("usage-providers.json");
        let prefs = match std::fs::read(&path) {
            Ok(bytes) => migrate_preferences(&bytes),
            Err(_) => {
                let prefs = std::fs::read(dir.join("installed-usage-enabled.json"))
                    .map(|b| migrate_preferences(&b))
                    .unwrap_or_default();
                let _ = std::fs::create_dir_all(&dir);
                if let Ok(bytes) = serde_json::to_vec(&prefs) {
                    let _ = std::fs::write(&path, bytes);
                }
                prefs
            }
        };
        *PREFERENCES.lock().unwrap_or_else(|p| p.into_inner()) = prefs;
        ENABLED.store(prefs.claude || prefs.codex, Ordering::Relaxed);
        let _ = SETTINGS_PATH.set(path);
    }
    std::thread::spawn(move || {
        let mut last = Instant::now() - Duration::from_secs(300);
        loop {
            if ENABLED.load(Ordering::Relaxed)
                && ({
                    let mut refresh = REFRESH.lock().unwrap_or_else(|p| p.into_inner());
                    std::mem::take(&mut *refresh)
                } || last.elapsed() >= Duration::from_secs(300))
            {
                let ticket = REQUESTED.load(Ordering::SeqCst);
                let epoch = EPOCH.load(Ordering::SeqCst);
                let previous = METERS.lock().unwrap_or_else(|p| p.into_inner()).clone();
                let mut meters = Vec::new();
                for name in ["claude", "codex"] {
                    if !provider_enabled(name) || executable(name).is_err() {
                        continue;
                    }
                    match probe(name) {
                        Ok(values) => meters.extend(values),
                        Err(ProbeError::NotConnected) => {}
                        Err(ProbeError::Failed(error)) => {
                            meters.extend(after_failed_probe(name, &previous, &error))
                        }
                    }
                }
                let committed = {
                    let mut cache = METERS.lock().unwrap_or_else(|p| p.into_inner());
                    if ENABLED.load(Ordering::SeqCst) && epoch == EPOCH.load(Ordering::SeqCst) {
                        *cache = meters;
                        true
                    } else {
                        false
                    }
                };
                if committed {
                    let _ = app.emit("usage", crate::usage::current_snapshot());
                }
                COMPLETED.store(ticket, Ordering::SeqCst);
                last = Instant::now();
            }
            let refresh = REFRESH.lock().unwrap_or_else(|p| p.into_inner());
            if ENABLED.load(Ordering::Relaxed) {
                let due = Duration::from_secs(300).saturating_sub(last.elapsed());
                let _ = WAKE
                    .wait_timeout_while(refresh, due, |refresh| !*refresh)
                    .unwrap_or_else(|p| p.into_inner());
            } else {
                let _guard = WAKE
                    .wait_while(refresh, |refresh| {
                        !ENABLED.load(Ordering::Relaxed) || !*refresh
                    })
                    .unwrap_or_else(|p| p.into_inner());
            }
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preference_migration_and_independent_opt_ins() {
        let old = migrate_preferences(b"true");
        assert!(old.claude && old.codex);
        for bytes in [b"false".as_slice(), b"invalid", b"{}"] {
            let p = migrate_preferences(bytes);
            assert!(!p.claude && !p.codex);
        }
        let p = migrate_preferences(br#"{"claude":false,"codex":true}"#);
        assert!(!p.claude && p.codex);
    }
    #[test]
    fn detection_only_checks_executable_candidates() {
        let dir = std::env::temp_dir().join(format!("tuck-detect-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing.exe");
        let found = dir.join("fixture.exe");
        assert_eq!(
            detected_executable(vec![missing.clone(), dir.clone()]),
            None
        );
        std::fs::write(&found, b"fixture, never executed").unwrap();
        assert_eq!(
            detected_executable(vec![missing, found.clone()]),
            Some(found.clone())
        );
        std::fs::remove_file(found).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn codex_only_subscription_and_valid_quota_fields() {
        let account = json!({"account":{"type":"chatgpt","email":"PRIVATE"}});
        let limits = json!({"rateLimits":{"primary":{"usedPercent":22,"windowDurationMins":300,"resetsAt":1900000000}}});
        let meters = parse_codex(&account, &limits).unwrap();
        assert_eq!(meters[0].used_pct, Some(22.0));
        assert!(!serde_json::to_string(&meters).unwrap().contains("PRIVATE"));
        assert!(parse_codex(&json!({"account":{"type":"apiKey"}}), &limits).is_err());
    }
    #[test]
    fn claude_control_quota_requires_actual_usage_and_reset() {
        let values=parse_claude(&json!({"rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":35,"resets_at":"2030-01-01T00:00:00Z"}}})).unwrap();
        assert_eq!(values[0].used_pct, Some(35.0));
        assert!(parse_claude(&json!({"rate_limits":{"five_hour":{"utilization":35}}})).is_err());
    }
    #[test]
    fn missing_or_signed_out_clients_are_not_connected() {
        assert!(matches!(
            probe("tuck-test-client-that-is-not-installed"),
            Err(ProbeError::NotConnected)
        ));
        assert!(matches!(
            codex_account(&json!({"account":null})),
            Err(ProbeError::NotConnected)
        ));
        assert!(matches!(
            codex_account(&json!({"account":{"type":"apiKey"}})),
            Err(ProbeError::NotConnected)
        ));
        assert!(codex_account(&json!({"account":{"type":"chatgpt"}})).is_ok());
        assert!(matches!(
            claude_usage(&json!({"rate_limits_available":false})),
            Err(ProbeError::NotConnected)
        ));
    }
    #[test]
    fn a_connected_client_without_quota_data_is_a_failure_not_absent() {
        assert!(matches!(
            claude_usage(&json!({"rate_limits_available":true})),
            Err(ProbeError::Failed(_))
        ));
    }
    #[test]
    fn failed_probe_keeps_real_values_stale_and_never_invents_any() {
        let account = json!({"account":{"type":"chatgpt"}});
        let limits = json!({"rateLimits":{"primary":{"usedPercent":22,"windowDurationMins":300,"resetsAt":1900000000}}});
        let previous = parse_codex(&account, &limits).unwrap();
        let kept = after_failed_probe("codex", &previous, "timed out");
        assert_eq!(kept[0].used_pct, Some(22.0));
        assert!(kept[0].stale);
        assert_eq!(kept[0].status.as_deref(), Some("error"));
        // Another provider's data is not borrowed: it shows an empty error reading
        let empty = after_failed_probe("claude", &previous, "timed out");
        assert_eq!(empty.len(), 1);
        assert_eq!(empty[0].provider, "claude");
        assert_eq!(empty[0].used_pct, None);
        assert_eq!(empty[0].status.as_deref(), Some("error"));
    }
    #[test]
    #[ignore = "Explicit local installed-client read-only smoke"]
    fn installed_clients_live() {
        let meters = probe("codex").expect("Installed Codex quota probe");
        assert!(!meters.is_empty());
        println!("codex: {} quota windows", meters.len());
        let mut client = Client::start("claude").expect("Installed Claude control client");
        let init = client
            .request(1, "", json!({"subtype":"initialize"}), true)
            .expect("Claude initialize");
        // Whitelist only non-identifying connection metadata. Never print the full response.
        println!(
            "claude connection: subscription={}, tokenSource={}, apiProvider={}",
            init["account"]["subscriptionType"]
                .as_str()
                .unwrap_or("unreported"),
            init["account"]["tokenSource"]
                .as_str()
                .unwrap_or("unreported"),
            init["account"]["apiProvider"]
                .as_str()
                .unwrap_or("unreported")
        );
        let usage = client
            .request(
                2,
                "",
                json!({"subtype":"get_usage","skip_behaviors":true}),
                true,
            )
            .expect("Claude usage control request");
        match parse_claude(&usage){Ok(meters)=>println!("claude: {} quota windows",meters.len()),Err(_)=>println!("claude: control request succeeded; subscription quota unavailable for this connection")}
    }
}
