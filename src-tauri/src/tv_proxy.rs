//! Local HLS proxy for in-app TV playback.
//!
//! TV channels play in an embedded `<video>` element (via hls.js), not a native
//! window. A webview can't touch raw IPTV streams directly: the app's CSP blocks
//! cross-origin requests, most streams send no CORS headers, many require a
//! specific `User-Agent`/`Referer`, and plenty are plain HTTP (mixed content).
//!
//! This tiny loopback server launders all of that. It fetches the upstream
//! playlist/segments natively (reqwest, with the required headers), rewrites the
//! playlist so every segment and child-playlist URL routes back through the
//! proxy, and serves everything from `http://127.0.0.1:<port>` with permissive
//! CORS — an origin the webview is allowed to load (localhost is exempt from
//! mixed-content blocking).
//!
//! Endpoints:
//! - `GET /hls?u=<url>&ua=<ua>&r=<ref>` — fetch + rewrite an M3U8 playlist.
//! - `GET /seg?u=<url>&ua=<ua>&r=<ref>` — stream a segment / key / media file.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tiny_http::{Header, Response, Server, StatusCode};
use url::Url;

/// Handle to the running proxy (managed Tauri state). Playback URLs are built
/// against `port`.
pub struct TvProxy {
    pub port: u16,
}

impl TvProxy {
    /// The proxied playback URL for a stream + its optional headers. hls.js /
    /// `<video>` load this instead of the raw stream.
    pub fn stream_url(&self, upstream: &str, user_agent: Option<&str>, referrer: Option<&str>) -> String {
        let mut url = format!("http://127.0.0.1:{}/hls?u={}", self.port, enc(upstream));
        if let Some(ua) = user_agent.filter(|s| !s.is_empty()) {
            url.push_str(&format!("&ua={}", enc(ua)));
        }
        if let Some(r) = referrer.filter(|s| !s.is_empty()) {
            url.push_str(&format!("&r={}", enc(r)));
        }
        url
    }
}

/// Start the proxy on an ephemeral loopback port. Each request is handled on its
/// own thread so segment fetches run concurrently (hls.js pulls several at once).
pub fn start() -> Option<TvProxy> {
    let server = Server::http("127.0.0.1:0").ok()?;
    let port = server.server_addr().to_ip()?.port();
    let client = Arc::new(
        reqwest::blocking::Client::builder()
            // Fail fast on dead/geo-blocked hosts (→ the error card, not a long
            // hang), but allow a slow live segment to finish downloading.
            .connect_timeout(Duration::from_secs(6))
            .timeout(Duration::from_secs(20))
            // Low-latency forwarding + reuse warm keep-alive connections across
            // the manifest → variant → segment chain (avoids re-doing DNS/TLS).
            .tcp_nodelay(true)
            .pool_max_idle_per_host(8)
            .build()
            .ok()?,
    );

    std::thread::Builder::new()
        .name("hm-tv-proxy".into())
        .spawn(move || {
            for request in server.incoming_requests() {
                let client = Arc::clone(&client);
                // Detached per-request thread: concurrent segment fetches.
                std::thread::spawn(move || handle(&client, port, request));
            }
        })
        .ok()?;
    Some(TvProxy { port })
}

fn handle(client: &reqwest::blocking::Client, port: u16, request: tiny_http::Request) {
    let raw = request.url().to_string();
    let (path, query) = raw.split_once('?').unwrap_or((raw.as_str(), ""));
    let params = parse_query(query);
    let Some(upstream) = params.get("u") else {
        respond(request, 400, "text/plain", b"missing url".to_vec(), None);
        return;
    };
    let ua = params.get("ua").map(String::as_str);
    let referer = params.get("r").map(String::as_str);

    match path {
        "/hls" => serve_playlist(client, port, upstream, ua, referer, request),
        "/seg" => serve_segment(client, upstream, ua, referer, request),
        _ => respond(request, 404, "text/plain", b"not found".to_vec(), None),
    }
}

/// Fetch an M3U8 and rewrite every URI (segments, keys, child playlists) to
/// route back through the proxy, resolving relative URIs against the playlist.
fn serve_playlist(
    client: &reqwest::blocking::Client,
    port: u16,
    upstream: &str,
    ua: Option<&str>,
    referer: Option<&str>,
    request: tiny_http::Request,
) {
    let Some(body) = fetch_text(client, upstream, ua, referer) else {
        respond(request, 502, "text/plain", b"upstream fetch failed".to_vec(), None);
        return;
    };
    let base = Url::parse(upstream).ok();
    let mut out = String::with_capacity(body.len() + 256);

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }
        if trimmed.starts_with('#') {
            out.push_str(&rewrite_tag_uri(trimmed, base.as_ref(), port, ua, referer));
            out.push('\n');
        } else {
            // A media segment or a child (variant) playlist URI.
            let abs = resolve(base.as_ref(), trimmed);
            out.push_str(&proxied(port, &abs, ua, referer));
            out.push('\n');
        }
    }

    respond(
        request,
        200,
        "application/vnd.apple.mpegurl",
        out.into_bytes(),
        Some(upstream),
    );
}

/// Stream a segment / key / media file straight through — piping the upstream
/// body as it arrives (no download-then-forward buffering), preserving its
/// content-type. Lower time-to-first-byte and memory per segment.
fn serve_segment(
    client: &reqwest::blocking::Client,
    upstream: &str,
    ua: Option<&str>,
    referer: Option<&str>,
    request: tiny_http::Request,
) {
    let mut req = client.get(upstream);
    if let Some(ua) = ua {
        req = req.header(reqwest::header::USER_AGENT, ua);
    }
    if let Some(r) = referer {
        req = req.header(reqwest::header::REFERER, r);
    }
    let Ok(resp) = req.send() else {
        respond(request, 502, "text/plain", b"upstream fetch failed".to_vec(), None);
        return;
    };
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let len = resp.content_length().map(|l| l as usize);

    let headers: Vec<Header> = [
        ("Content-Type", content_type.as_str()),
        ("Access-Control-Allow-Origin", "*"),
        ("Cache-Control", "no-store"),
    ]
    .iter()
    .filter_map(|(n, v)| Header::from_bytes(n.as_bytes(), v.as_bytes()).ok())
    .collect();

    // `resp` implements `Read`; tiny_http pulls bytes from it and writes them to
    // the socket as they come (chunked when the length is unknown).
    let response = Response::new(StatusCode(status), headers, resp, len, None);
    let _ = request.respond(response);
}

/// Rewrite a `URI="…"` attribute inside a playlist tag (EXT-X-KEY / -MAP /
/// -MEDIA), leaving other tags untouched.
fn rewrite_tag_uri(
    line: &str,
    base: Option<&Url>,
    port: u16,
    ua: Option<&str>,
    referer: Option<&str>,
) -> String {
    let Some(start) = line.find("URI=\"") else {
        return line.to_string();
    };
    let val_start = start + 5;
    let Some(end_rel) = line[val_start..].find('"') else {
        return line.to_string();
    };
    let end = val_start + end_rel;
    let abs = resolve(base, &line[val_start..end]);
    format!("{}{}{}", &line[..val_start], proxied(port, &abs, ua, referer), &line[end..])
}

/// The proxied URL for an already-absolute upstream URL — routing playlists to
/// `/hls` (so they're rewritten too) and everything else to `/seg`.
fn proxied(port: u16, abs_url: &str, ua: Option<&str>, referer: Option<&str>) -> String {
    let path = if is_playlist(abs_url) { "hls" } else { "seg" };
    let mut out = format!("http://127.0.0.1:{port}/{path}?u={}", enc(abs_url));
    if let Some(ua) = ua {
        out.push_str(&format!("&ua={}", enc(ua)));
    }
    if let Some(r) = referer {
        out.push_str(&format!("&r={}", enc(r)));
    }
    out
}

fn is_playlist(u: &str) -> bool {
    let path = u.split(['?', '#']).next().unwrap_or(u);
    path.to_ascii_lowercase().ends_with(".m3u8")
}

fn resolve(base: Option<&Url>, uri: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    base.and_then(|b| b.join(uri).ok())
        .map(|u| u.to_string())
        .unwrap_or_else(|| uri.to_string())
}

fn fetch_text(
    client: &reqwest::blocking::Client,
    url: &str,
    ua: Option<&str>,
    referer: Option<&str>,
) -> Option<String> {
    let mut req = client.get(url);
    if let Some(ua) = ua {
        req = req.header(reqwest::header::USER_AGENT, ua);
    }
    if let Some(r) = referer {
        req = req.header(reqwest::header::REFERER, r);
    }
    let resp = req.send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().ok()
}

fn respond(
    request: tiny_http::Request,
    status: u16,
    content_type: &str,
    body: Vec<u8>,
    _upstream: Option<&str>,
) {
    let mut response = Response::from_data(body).with_status_code(status);
    for (name, value) in [
        ("Content-Type", content_type),
        ("Access-Control-Allow-Origin", "*"),
        ("Cache-Control", "no-store"),
    ] {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response.add_header(header);
        }
    }
    let _ = request.respond(response);
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn parse_query(query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_detection_ignores_query_and_fragment() {
        assert!(is_playlist("https://h/x/index.m3u8"));
        assert!(is_playlist("https://h/x/index.m3u8?token=abc"));
        assert!(!is_playlist("https://h/x/seg1.ts"));
        assert!(!is_playlist("https://h/x/key.bin?a=1"));
    }

    #[test]
    fn relative_uris_resolve_against_the_playlist() {
        let base = Url::parse("https://host.example/live/index.m3u8").ok();
        assert_eq!(resolve(base.as_ref(), "seg1.ts"), "https://host.example/live/seg1.ts");
        assert_eq!(resolve(base.as_ref(), "/abs/seg.ts"), "https://host.example/abs/seg.ts");
        assert_eq!(
            resolve(base.as_ref(), "https://cdn.example/s.ts"),
            "https://cdn.example/s.ts"
        );
    }

    #[test]
    fn proxied_routes_playlists_to_hls_and_segments_to_seg() {
        assert!(proxied(9000, "https://h/v.m3u8", None, None).contains("/hls?u="));
        assert!(proxied(9000, "https://h/s.ts", None, None).contains("/seg?u="));
        // Headers are carried through when present.
        let with_ua = proxied(9000, "https://h/s.ts", Some("UA/1"), None);
        assert!(with_ua.contains("&ua="));
    }

    #[test]
    fn key_uri_in_a_tag_is_rewritten() {
        let base = Url::parse("https://h/live/index.m3u8").ok();
        let line = r#"#EXT-X-KEY:METHOD=AES-128,URI="enc.key",IV=0x1"#;
        let out = rewrite_tag_uri(line, base.as_ref(), 9000, None, None);
        assert!(out.contains("/seg?u="));
        assert!(out.contains("METHOD=AES-128"));
        assert!(out.contains("IV=0x1"));
    }

    #[test]
    fn stream_url_builds_a_loopback_hls_url() {
        let proxy = TvProxy { port: 9000 };
        let u = proxy.stream_url("https://h/index.m3u8", Some("Moz/5"), None);
        assert!(u.starts_with("http://127.0.0.1:9000/hls?u="));
        assert!(u.contains("&ua="));
        assert!(!u.contains("&r="));
    }
}
