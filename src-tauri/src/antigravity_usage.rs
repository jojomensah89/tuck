//! Opt-in exception: only the exact ACP sign-in file. No logging or secret events.
use crate::usage::Meter;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{io::Read, path::PathBuf, time::Duration};
use zeroize::Zeroizing;
const ERROR: &str = "Antigravity quota unavailable";
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
        .map_err(|_| ERROR)?;
    if bytes.len() > MAX_FILE {
        return Err(ERROR);
    }
    let c: Credentials = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
    if [&c.client_id, &c.client_secret, &c.refresh_token]
        .iter()
        .any(|s| s.0.is_empty())
    {
        return Err(ERROR);
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
async fn response(request: reqwest::RequestBuilder) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    let request = request.build().map_err(|_| ERROR)?;
    if !allowed(request.url()) {
        return Err(ERROR);
    }
    let mut r = client()?.execute(request).await.map_err(|_| ERROR)?;
    if !r.status().is_success() {
        return Err(ERROR);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = r.chunk().await.map_err(|_| ERROR)? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(ERROR);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn parse_quota(v: &Value, models: bool) -> Vec<Meter> {
    let mut windows = Vec::new();
    if models {
        if let Some(map) = v["models"].as_object() {
            for model in map.values().take(128) {
                for key in ["quotaInfo", "weeklyQuotaInfo"] {
                    windows.push((&model[key], "Antigravity model quota"));
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
                windows.push((b, "Antigravity bucket quota"));
            }
        }
    }
    // Conservative single ring: most-used scoped allowance, never a general weekly.
    windows
        .into_iter()
        .filter_map(|(q, label)| {
            let remaining = q["remainingFraction"]
                .as_f64()
                .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))?;
            Some(Meter {
                id: "antigravity-scoped".into(),
                provider: "antigravity".into(),
                label: label.into(),
                used_pct: Some((1.0 - remaining) * 100.0),
                spent_usd: None,
                resets_at: crate::usage::parse_unix_timestamp(&q["resetTime"]),
                source: "official".into(),
                stale: false,
                status: Some("ok".into()),
                error: None,
            })
        })
        .max_by(|a, b| {
            a.used_pct
                .partial_cmp(&b.used_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .into_iter()
        .collect()
}
async fn fetch(c: Credentials) -> Result<Vec<Meter>, &'static str> {
    let http = client()?;
    let bytes = response(http.post("https://oauth2.googleapis.com/token").form(&[
        ("client_id", c.client_id.0.as_str()),
        ("client_secret", c.client_secret.0.as_str()),
        ("refresh_token", c.refresh_token.0.as_str()),
        ("grant_type", "refresh_token"),
    ]))
    .await?;
    let access: Access = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
    if access.access_token.0.is_empty() {
        return Err(ERROR);
    }
    drop(bytes);
    // The first host that answers ends the search, so the token is sent no more than needed.
    for (operation, models) in [
        ("retrieveUserQuotaSummary", false),
        ("fetchAvailableModels", true),
    ] {
        for host in [
            "daily-cloudcode-pa.googleapis.com",
            "cloudcode-pa.googleapis.com",
        ] {
            let body = if let Some(p) = &c.project_id {
                json!({"project":p.0.as_str()})
            } else {
                json!({})
            };
            if let Ok(bytes) = response(
                http.post(format!("https://{host}/v1internal:{operation}"))
                    .bearer_auth(access.access_token.0.as_str())
                    .json(&body),
            )
            .await
            {
                if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                    let meters = parse_quota(&v, models);
                    if !meters.is_empty() {
                        return Ok(meters);
                    }
                }
            }
        }
    }
    Err(ERROR)
}
pub fn probe(active: impl Fn() -> bool) -> Result<Vec<Meter>, &'static str> {
    // Gate before even resolving/opening the file; caller captures the enablement epoch.
    if !active() {
        return Err(ERROR);
    }
    let file = std::fs::File::open(path().ok_or(ERROR)?).map_err(|_| ERROR)?;
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
            assert_eq!(read_credentials(&b[..]).err(), Some(ERROR));
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
            Some(ERROR)
        );
        std::fs::remove_file(file).unwrap();
        assert_eq!(probe(|| false).unwrap_err(), ERROR);
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
    fn public_shapes_scoped_and_sanitized() {
        let summary = json!({"groups":[{"displayName":"FAKE_SECRET","buckets":[{"bucketId":"gemini-weekly","window":"weekly","remainingFraction":0.4,"resetTime":"2030-01-01T00:00:00Z"}]}]});
        let m = parse_quota(&summary, false);
        assert_eq!(m[0].used_pct, Some(60.0));
        let event = serde_json::to_string(&m).unwrap();
        assert!(!event.contains("FAKE_SECRET"));
        assert!(!event.contains("weekly"));
        assert_eq!(
            parse_quota(
                &json!({"models":{"gemini-test":{"quotaInfo":{"remainingFraction":0.25}}}}),
                true
            )[0]
            .used_pct,
            Some(75.0)
        );
        assert!(parse_quota(&json!({"buckets":[{"remainingFraction":2}]}), false).is_empty());
    }
}
