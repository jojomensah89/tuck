//! Opt-in exception: only the exact ACP sign-in file. No logging or secret events.
use crate::usage::Meter;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{io::Read, path::PathBuf, time::Duration};
use zeroize::Zeroizing;
// Every message is fixed text that says which step failed. None can hold a token or a reply.
const ERROR: &str = "Antigravity quota unavailable";
const FILE_ERROR: &str = "Antigravity's sign-in file couldn't be read";
const OFFLINE: &str = "Couldn't reach Google";
const SIGN_IN_REFUSED: &str = "Google refused the sign-in. Sign in to Antigravity again";
const NO_QUOTA: &str = "Google's reply had no quota in it";
// Google's private quota service refuses clients it doesn't recognise. Same value as T3 Code.
const USER_AGENT: &str =
    "antigravity/hub/2.24.0 (aidev_client; os_type=windows; arch=x86_64; cl=963137146)";
// UNVERIFIED Windows default; never search T3 profiles or alternative credential paths.
const UNVERIFIED_WINDOWS_TOKEN_PATH: &str = ".gemini/antigravity-acp/acp_token.json";
const MAX_FILE: usize = 16 * 1024;
struct Secret(Zeroizing<String>);
impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|s| Self(Zeroizing::new(s)))
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
    }
}
#[derive(Deserialize)]
struct Credentials {
    client_id: Secret,
    client_secret: Secret,
    refresh_token: Secret,
    project_id: Option<Secret>,
}
#[derive(Deserialize)]
struct Access {
    access_token: Secret,
}
fn path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(UNVERIFIED_WINDOWS_TOKEN_PATH))
}
pub fn installed() -> bool {
    path().is_some_and(|p| p.is_file())
}
fn read_credentials(reader: impl Read) -> Result<Credentials, &'static str> {
    let mut bytes = Zeroizing::new(Vec::new());
    reader
        .take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| FILE_ERROR)?;
    if bytes.len() > MAX_FILE {
        return Err(FILE_ERROR);
    }
    let c: Credentials = serde_json::from_slice(&bytes).map_err(|_| FILE_ERROR)?;
    if [&c.client_id, &c.client_secret, &c.refresh_token]
        .iter()
        .any(|s| s.0.is_empty())
    {
        return Err(FILE_ERROR);
    }
    Ok(c)
}
fn allowed(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "oauth2.googleapis.com"
                    | "daily-cloudcode-pa.googleapis.com"
                    | "cloudcode-pa.googleapis.com"
            )
        )
}
fn client() -> Result<reqwest::Client, &'static str> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| ERROR)
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Failure {
    Offline,
    Refused(u16),
    Other,
}
fn quota_refused(status: u16) -> &'static str {
    match status {
        401 => "Google refused the quota request (401)",
        403 => "Google refused the quota request (403)",
        404 => "Google refused the quota request (404)",
        429 => "Google refused the quota request (429)",
        400..=499 => "Google refused the quota request",
        _ => "Google's quota service is having trouble",
    }
}
async fn response(request: reqwest::RequestBuilder) -> Result<Zeroizing<Vec<u8>>, Failure> {
    let request = request.build().map_err(|_| Failure::Other)?;
    if !allowed(request.url()) {
        return Err(Failure::Other);
    }
    let client = client().map_err(|_| Failure::Other)?;
    let mut r = client
        .execute(request)
        .await
        .map_err(|_| Failure::Offline)?;
    if !r.status().is_success() {
        return Err(Failure::Refused(r.status().as_u16()));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = r.chunk().await.map_err(|_| Failure::Offline)? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(Failure::Other);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn parse_quota(v: &Value, models: bool, now: i64) -> Vec<Meter> {
    let mut windows = Vec::new();
    if models {
        if let Some(map) = v["models"].as_object() {
            for model in map.values().take(128) {
                for key in ["quotaInfo", "weeklyQuotaInfo"] {
                    windows.push((&model[key], key == "weeklyQuotaInfo"));
                }
            }
        }
    } else {
        let groups = v["groups"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let grouped = groups
            .iter()
            .any(|g| g["buckets"].as_array().is_some_and(|b| !b.is_empty()));
        let buckets: Vec<&Value> = if grouped {
            groups
                .iter()
                .flat_map(|g| g["buckets"].as_array().into_iter().flatten())
                .collect()
        } else {
            v["buckets"].as_array().into_iter().flatten().collect()
        };
        for b in buckets.into_iter().take(128) {
            if b["disabled"] != true {
                windows.push((b, weekly_bucket(b, now)));
            }
        }
    }
    let mut meters = Vec::new();
    let candidates = windows
        .into_iter()
        .filter_map(|(q, weekly)| {
            let remaining = q["remainingFraction"]
                .as_f64()
                .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))?;
            Some(Meter {
                id: if weekly {
                    "antigravity-weekly"
                } else {
                    "antigravity-short"
                }
                .into(),
                provider: "antigravity".into(),
                // No reply text crosses the native bridge. Short duration uses the
                // same 5h convention as Claude/Codex; reset is in the tooltip.
                label: if weekly {
                    "Antigravity weekly"
                } else {
                    "Antigravity 5 h"
                }
                .into(),
                used_pct: Some((1.0 - remaining) * 100.0),
                spent_usd: None,
                resets_at: crate::usage::parse_unix_timestamp(&q["resetTime"]),
                source: "official".into(),
                stale: false,
                status: Some("ok".into()),
                error: None,
            })
        })
        .collect();
    merge_windows(&mut meters, candidates);
    meters
}
// Only explicit window fields/identifiers or a distant reset imply a weekly bucket.
fn weekly_bucket(q: &Value, now: i64) -> bool {
    [
        "window",
        "windowType",
        "quotaType",
        "bucketId",
        "duration",
        "resetPeriod",
    ]
    .iter()
    .filter_map(|key| q[*key].as_str())
    .any(|value| {
        let value = value.to_ascii_lowercase();
        value.contains("week") || matches!(value.as_str(), "7d" | "7 days" | "604800s")
    }) || crate::usage::parse_unix_timestamp(&q["resetTime"])
        .is_some_and(|reset| reset.saturating_sub(now) > 2 * 86400)
}
fn merge_windows(meters: &mut Vec<Meter>, candidates: Vec<Meter>) {
    for candidate in candidates {
        if let Some(current) = meters.iter_mut().find(|m| m.id == candidate.id) {
            if candidate.used_pct > current.used_pct {
                *current = candidate;
            }
        } else {
            meters.push(candidate);
        }
    }
    meters.sort_by_key(|m| m.id == "antigravity-weekly");
}
/// An empty project ID is the same as none: the service treats a blank one as a real value.
fn quota_body(project: Option<&Secret>) -> Value {
    match project.filter(|p| !p.0.is_empty()) {
        Some(p) => json!({"project":p.0.as_str()}),
        None => json!({}),
    }
}
async fn fetch(c: Credentials) -> Result<Vec<Meter>, &'static str> {
    let http = client()?;
    let bytes = response(http.post("https://oauth2.googleapis.com/token").form(&[
        ("client_id", c.client_id.0.as_str()),
        ("client_secret", c.client_secret.0.as_str()),
        ("refresh_token", c.refresh_token.0.as_str()),
        ("grant_type", "refresh_token"),
    ]))
    .await
    .map_err(|failure| match failure {
        Failure::Offline => OFFLINE,
        Failure::Refused(_) => SIGN_IN_REFUSED,
        Failure::Other => ERROR,
    })?;
    let access: Access = serde_json::from_slice(&bytes).map_err(|_| SIGN_IN_REFUSED)?;
    if access.access_token.0.is_empty() {
        return Err(SIGN_IN_REFUSED);
    }
    drop(bytes);
    let mut failed = None;
    let mut meters = Vec::new();
    let now = chrono::Utc::now().timestamp();
    for (operation, models) in [
        ("retrieveUserQuotaSummary", false),
        ("fetchAvailableModels", true),
    ] {
        // The hosts can report different quota groups, so read both and keep the most used.
        for host in [
            "daily-cloudcode-pa.googleapis.com",
            "cloudcode-pa.googleapis.com",
        ] {
            match response(
                http.post(format!("https://{host}/v1internal:{operation}"))
                    .bearer_auth(access.access_token.0.as_str())
                    .header(reqwest::header::USER_AGENT, USER_AGENT)
                    .json(&quota_body(c.project_id.as_ref())),
            )
            .await
            {
                Ok(bytes) => {
                    if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                        merge_windows(&mut meters, parse_quota(&v, models, now));
                    }
                }
                Err(failure) => failed = Some(failure),
            }
        }
        // Both hosts may carry different model groups. Once both window classes
        // are present, no fallback operation is needed.
        if meters.len() == 2 {
            return Ok(meters);
        }
    }
    if !meters.is_empty() {
        return Ok(meters);
    }
    Err(match failed {
        Some(Failure::Offline) => OFFLINE,
        Some(Failure::Refused(status)) => quota_refused(status),
        Some(Failure::Other) => ERROR,
        None => NO_QUOTA,
    })
}
pub fn probe(active: impl Fn() -> bool) -> Result<Vec<Meter>, &'static str> {
    // Gate before even resolving/opening the file; caller captures the enablement epoch.
    if !active() {
        return Err(ERROR);
    }
    let file = std::fs::File::open(path().ok_or(FILE_ERROR)?).map_err(|_| FILE_ERROR)?;
    let c = read_credentials(file)?;
    if !active() {
        return Err(ERROR);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| ERROR)?;
    runtime.block_on(async {
        let work = fetch(c);
        tokio::pin!(work);
        let deadline = tokio::time::sleep(Duration::from_secs(30));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                result = &mut work => return if active() { result } else { Err(ERROR) },
                _ = &mut deadline => return Err(ERROR),
                _ = tokio::time::sleep(Duration::from_millis(25)) => if !active() { return Err(ERROR); }
            }
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fake_files_and_redaction() {
        let fake = br#"{"client_id":"FAKE_ID","client_secret":"FAKE_SECRET","refresh_token":"FAKE_REFRESH"}"#;
        let c = read_credentials(&fake[..]).unwrap();
        assert_eq!(format!("{:?}", c.refresh_token), "");
        for b in [
            b"FAKE_REFRESH".to_vec(),
            vec![b'x'; MAX_FILE + 1],
            b"{}".to_vec(),
        ] {
            assert_eq!(read_credentials(&b[..]).err(), Some(FILE_ERROR));
        }
        assert_eq!(probe(|| false).unwrap_err(), ERROR); // never opens any real file
        assert!(!ERROR.contains("FAKE"));
    }
    #[test]
    fn fake_file_and_cancellation_gate() {
        let file = std::env::temp_dir().join(format!("tuck-fake-acp-{}.json", std::process::id()));
        std::fs::write(&file, br#"{"client_id":"FAKE_ID","client_secret":"FAKE_SECRET","refresh_token":"FAKE_REFRESH"}"#).unwrap();
        assert!(read_credentials(std::fs::File::open(&file).unwrap()).is_ok());
        std::fs::write(&file, vec![b'x'; MAX_FILE + 1]).unwrap();
        assert_eq!(
            read_credentials(std::fs::File::open(&file).unwrap()).err(),
            Some(FILE_ERROR)
        );
        std::fs::remove_file(file).unwrap();
        assert_eq!(probe(|| false).unwrap_err(), ERROR);
    }
    #[test]
    fn a_blank_project_is_left_out_of_the_quota_request() {
        let with = |file: &[u8]| {
            let c = read_credentials(file).unwrap();
            quota_body(c.project_id.as_ref())
        };
        let base =
            r#""client_id":"FAKE_ID","client_secret":"FAKE_SECRET","refresh_token":"FAKE_REFRESH""#;
        assert_eq!(with(format!("{{{base}}}").as_bytes()), json!({}));
        assert_eq!(
            with(format!(r#"{{{base},"project_id":""}}"#).as_bytes()),
            json!({})
        );
        assert_eq!(
            with(format!(r#"{{{base},"project_id":"fake-project"}}"#).as_bytes()),
            json!({"project":"fake-project"})
        );
    }
    #[test]
    fn failure_messages_name_the_step_and_hold_nothing_from_a_reply() {
        assert!(quota_refused(403).contains("403"));
        assert!(quota_refused(418).contains("refused"));
        assert!(quota_refused(503).contains("trouble"));
        assert!(USER_AGENT.starts_with("antigravity/"));
        for message in [
            ERROR,
            FILE_ERROR,
            OFFLINE,
            SIGN_IN_REFUSED,
            NO_QUOTA,
            quota_refused(403),
        ] {
            assert!(!message.contains("FAKE"));
        }
    }
    #[test]
    fn redirects_are_not_followed() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let _ = socket.read(&mut bytes);
            socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://example.invalid/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        // Local HTTP fixture only; production is HTTPS-only, with this same redirect policy.
        let r = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
            .get(format!("http://{address}"))
            .send()
            .unwrap();
        assert_eq!(r.status().as_u16(), 302);
        server.join().unwrap();
    }
    #[test]
    fn exact_hosts_only() {
        for s in [
            "http://oauth2.googleapis.com/token",
            "https://oauth2.googleapis.com.evil.test/token",
            "https://www.googleapis.com/oauth2/v2/userinfo",
            "https://oauth2.googleapis.com:444/token",
        ] {
            assert!(!allowed(&s.parse().unwrap()));
        }
        assert!(allowed(
            &"https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels"
                .parse()
                .unwrap()
        ));
    }
    #[test]
    fn fake_models_select_most_used_per_window() {
        let fixture = json!({"models": {
            "fake-a": {"quotaInfo": {"remainingFraction": 0.25}, "weeklyQuotaInfo": {"remainingFraction": 0.7}},
            "fake-b": {"quotaInfo": {"remainingFraction": 0.6}, "weeklyQuotaInfo": {"remainingFraction": 0.1}}
        }});
        let meters = parse_quota(&fixture, true, 1000);
        assert_eq!(meters.len(), 2);
        assert_eq!(meters[0].label, "Antigravity 5 h");
        assert_eq!(meters[0].used_pct, Some(75.0));
        assert_eq!(meters[1].label, "Antigravity weekly");
        assert_eq!(meters[1].used_pct, Some(90.0));
    }
    #[test]
    fn fake_buckets_classify_weekly_and_ignore_invalid_or_disabled() {
        let now = 1_900_000_000;
        let fixture = json!({"buckets": [
            {"remainingFraction": 0.5, "resetTime": now + 3600},
            {"remainingFraction": 0.4, "resetTime": now + 3 * 86400},
            {"remainingFraction": 0.2, "windowType": "WEEKLY", "resetTime": now + 30},
            {"remainingFraction": 0.0, "window": "weekly", "disabled": true},
            {"remainingFraction": -1, "window": "weekly"}
        ]});
        let meters = parse_quota(&fixture, false, now);
        assert_eq!(meters.len(), 2);
        assert_eq!(meters[0].used_pct, Some(50.0));
        assert_eq!(meters[1].used_pct, Some(80.0));
        // Missing or invalid weekly data must not synthesize a second meter.
        let single = parse_quota(
            &json!({"models":{"fake":{
                "quotaInfo":{"remainingFraction":0.5}, "weeklyQuotaInfo":{"remainingFraction":2}
            }}}),
            true,
            now,
        );
        assert_eq!(single.len(), 1);
    }
    #[test]
    fn fake_operations_and_hosts_merge_without_losing_short_window() {
        let mut meters = parse_quota(&json!({"buckets":[{"remainingFraction":0.1}]}), false, 0);
        merge_windows(
            &mut meters,
            parse_quota(
                &json!({"models":{"fake":{
                    "quotaInfo":{"remainingFraction":0.8}, "weeklyQuotaInfo":{"remainingFraction":0.4}
                }}}),
                true,
                0,
            ),
        );
        merge_windows(
            &mut meters,
            parse_quota(
                &json!({"models":{"other-fake":{
                    "weeklyQuotaInfo":{"remainingFraction":0.2}
                }}}),
                true,
                0,
            ),
        );
        assert_eq!(meters.len(), 2);
        assert_eq!(meters[0].used_pct, Some(90.0));
        assert_eq!(meters[1].used_pct, Some(80.0));
    }
    #[test]
    fn public_shapes_scoped_and_sanitized() {
        let summary = json!({"groups":[{"displayName":"FAKE_SECRET","buckets":[{"bucketId":"gemini-weekly","window":"weekly","remainingFraction":0.4,"resetTime":"2030-01-01T00:00:00Z"}]}]});
        let m = parse_quota(&summary, false, 0);
        assert_eq!(m[0].used_pct, Some(60.0));
        let event = serde_json::to_string(&m).unwrap();
        assert!(!event.contains("FAKE_SECRET"));
        assert_eq!(m[0].label, "Antigravity weekly");
        assert_eq!(
            parse_quota(
                &json!({"models":{"gemini-test":{"quotaInfo":{"remainingFraction":0.25}}}}),
                true,
                0
            )[0]
            .used_pct,
            Some(75.0)
        );
        assert!(parse_quota(&json!({"buckets":[{"remainingFraction":2}]}), false, 0).is_empty());
    }
}
