//! Ephemeral, capability-URL shelf downloads. No listener exists outside a session.
use axum::{
    body::Body,
    http::{Response, StatusCode},
};
use std::{convert::Infallible, net::Ipv4Addr, sync::Mutex, time::Duration};
use subtle::ConstantTimeEq;
static EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SESSION: Mutex<Option<tokio::task::JoinHandle<()>>> = Mutex::new(None);

fn private(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
}
fn select_lan(ips: impl IntoIterator<Item = Ipv4Addr>) -> Option<Ipv4Addr> {
    ips.into_iter().find(|ip| private(*ip))
}
fn token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
#[derive(Debug, PartialEq)]
enum Route {
    Listing,
    File(usize),
    Missing,
}
fn route(path: &str, token: &str, ids: &[String], active: bool) -> Route {
    let mut parts = path.split('/');
    if parts.next() != Some("") {
        return Route::Missing;
    }
    let supplied = parts.next().unwrap_or("");
    // Compare fixed-size buffers even for malformed lengths.
    let mut candidate = [0u8; 64];
    for (to, from) in candidate.iter_mut().zip(supplied.bytes()) {
        *to = from;
    }
    let equal = candidate.ct_eq(token.as_bytes()).unwrap_u8() == 1;
    if !active || !equal || supplied.len() != token.len() {
        return Route::Missing;
    }
    match (parts.next(), parts.next()) {
        (Some(""), None) | (None, None) => Route::Listing,
        (Some(id), None) => ids
            .iter()
            .position(|v| v == id)
            .map(Route::File)
            .unwrap_or(Route::Missing),
        _ => Route::Missing,
    }
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn response(status: StatusCode, body: Body, content_type: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Cache-Control", "no-store")
        .header("Referrer-Policy", "no-referrer")
        .header("X-Content-Type-Options", "nosniff")
        .header(
            "Content-Security-Policy",
            "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'",
        )
        .body(body)
        .unwrap()
}
async fn request(
    path: &str,
    key: &str,
    files: &[(String, String)],
    active: bool,
) -> Response<Body> {
    let missing = || response(StatusCode::NOT_FOUND, Body::empty(), "text/plain");
    let shelf = crate::extensions::snapshot().shelf;
    let ids: Vec<_> = files.iter().map(|(id, _)| id.clone()).collect();
    match route(path, key, &ids, active) {
        Route::Missing => missing(),
        Route::Listing => {
            let mut html = String::from("<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>Tuck files</title><style>body{font:16px system-ui;background:#101012;color:#fff;max-width:38rem;margin: auto;padding:24px}h1{font-size:28px}article{background:#242426;border-radius:12px;padding:16px;margin:12px 0;overflow-wrap:anywhere}small{display:block;color:#aaa;margin:8px 0}a{color:white;display:inline-block;padding:12px;border:1px solid #666;border-radius:8px}</style><h1>Tuck files</h1>");
            for (id, shelf_id) in files {
                if let Some(item) = shelf.iter().find(|i| &i.id == shelf_id) {
                    html.push_str(&format!("<article>{}<small>{} bytes</small><a download href='/{}/{}'>Download</a></article>",escape(&item.name),item.bytes,key,id));
                }
            }
            response(StatusCode::OK, Body::from(html), "text/html; charset=utf-8")
        }
        Route::File(index) => {
            let Some(item) = shelf.iter().find(|i| i.id == files[index].1) else {
                return missing();
            };
            let Ok(file) = tokio::fs::File::open(&item.path).await else {
                return missing();
            };
            let mut result = response(
                StatusCode::OK,
                Body::from_stream(tokio_util::io::ReaderStream::new(file)),
                "application/octet-stream",
            );
            // RFC 5987 filename; never inject the original name into a header.
            let name: String = item
                .name
                .as_bytes()
                .iter()
                .map(|b| format!("%{b:02X}"))
                .collect();
            result.headers_mut().insert(
                "Content-Disposition",
                format!("attachment; filename*=UTF-8''{name}")
                    .parse()
                    .unwrap(),
            );
            result
        }
    }
}
pub fn stop() -> u64 {
    let epoch = EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    if let Some(task) = SESSION.lock().unwrap_or_else(|e| e.into_inner()).take() {
        task.abort();
    }
    epoch
}
#[tauri::command]
pub fn phone_stop() {
    stop();
}
#[derive(serde::Serialize)]
pub struct Transfer {
    svg: String,
}
#[tauri::command]
pub async fn phone_start() -> Result<Transfer, String> {
    let epoch = stop();
    let ip = select_lan(
        if_addrs::get_if_addrs()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter_map(|i| match i.ip() {
                std::net::IpAddr::V4(ip) => Some(ip),
                _ => None,
            }),
    )
    .ok_or("No Wi-Fi or network connection")?;
    let key = token()?;
    let files = crate::extensions::snapshot()
        .shelf
        .into_iter()
        .map(|i| Ok((token()?, i.id)))
        .collect::<Result<Vec<_>, String>>()?;
    let listener = tokio::net::TcpListener::bind((ip, 0))
        .await
        .map_err(|e| e.to_string())?;
    let url = format!(
        "http://{}:{}/{}/",
        ip,
        listener.local_addr().map_err(|e| e.to_string())?.port(),
        key
    );
    let svg = qrcode::QrCode::new(url)
        .map_err(|e| e.to_string())?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(160, 160)
        .build();
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        let deadline = tokio::time::sleep(Duration::from_secs(600));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                Some(_) = connections.join_next(), if !connections.is_empty() => {},
                accepted = listener.accept() => {
                    let Ok((socket,_)) = accepted else { break; };
                    let key = key.clone(); let files = files.clone();
                    connections.spawn(async move {
                        let service = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                            let key = key.clone(); let files = files.clone();
                            async move { Ok::<_,Infallible>(if req.method() == hyper::Method::GET { request(req.uri().path(),&key,&files,true).await } else { response(StatusCode::NOT_FOUND,Body::empty(),"text/plain") }) }
                        });
                        let _ = tokio::time::timeout(Duration::from_secs(600), hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(socket),service)).await;
                    });
                }
            }
        }
        // JoinSet drop aborts every active connection, including downloads.
    });
    let mut session = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    if epoch != EPOCH.load(std::sync::atomic::Ordering::SeqCst) {
        task.abort();
        return Err("Transfer closed".into());
    }
    *session = Some(task);
    Ok(Transfer { svg })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_ranges() {
        for ip in ["10.0.0.1", "172.16.0.1", "172.31.255.254", "192.168.1.1"] {
            assert!(private(ip.parse().unwrap()));
        }
        for ip in [
            "127.0.0.1",
            "169.254.1.1",
            "172.15.0.1",
            "172.32.0.1",
            "8.8.8.8",
        ] {
            assert!(!private(ip.parse().unwrap()));
        }
        assert_eq!(select_lan([Ipv4Addr::LOCALHOST]), None);
    }
    #[test]
    fn routing_is_closed_by_default() {
        let key = "a".repeat(64);
        let ids = vec!["opaque".into()];
        for path in ["/", "/wrong/", &format!("/{key}/unknown")] {
            assert_eq!(route(path, &key, &ids, true), Route::Missing);
        }
        assert_eq!(
            route(&format!("/{key}/opaque"), &key, &ids, false),
            Route::Missing
        );
        assert_eq!(
            route(&format!("/{key}/opaque"), &key, &ids, true),
            Route::File(0)
        );
    }
    #[tokio::test]
    async fn unauthorized_requests_are_404() {
        let key = "a".repeat(64);
        let files = vec![("opaque".into(), "shelf-id".into())];
        for path in ["/", "/wrong/", &format!("/{key}/unknown")] {
            assert_eq!(
                request(path, &key, &files, true).await.status(),
                StatusCode::NOT_FOUND
            );
        }
        assert_eq!(
            request(&format!("/{key}/"), &key, &files, false)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    #[tokio::test]
    async fn stop_closes_listener() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
        stop();
        tokio::task::yield_now().await;
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }
    #[test]
    fn random_tokens() {
        let a = token().unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, token().unwrap());
    }
}
