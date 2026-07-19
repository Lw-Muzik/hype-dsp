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
    /// The proxied URL for a progressive media file (a YouTube Music video-only
    /// rendition), seekable via forwarded `Range`.
    ///
    /// The `<video>` element can't reach googlevideo itself: the CSP forbids the
    /// origin, and the CDN wants the User-Agent of the client that resolved the
    /// URL — which no element can set. Same laundering as TV, different shape of
    /// body.
    pub fn video_url(&self, upstream: &str, user_agent: Option<&str>) -> String {
        let mut url = format!("http://127.0.0.1:{}/video?u={}", self.port, enc(upstream));
        if let Some(ua) = user_agent.filter(|s| !s.is_empty()) {
            url.push_str(&format!("&ua={}", enc(ua)));
        }
        url
    }

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
        "/video" => serve_video(client, upstream, ua, request),
        _ => respond(request, 404, "text/plain", b"not found".to_vec(), None),
    }
}

/// How long a single video body may take to arrive.
///
/// The client-wide 20s deadline cannot apply here: reqwest measures it "until the
/// response body has finished", and a video-only rendition is tens of MB, so a
/// whole-body request was being **cut off mid-file at 20 seconds** — which the
/// element shows as a video that stalls and never recovers. Segments are small
/// and keep the tight deadline; this one needs to bound a transfer, not a click.
///
/// Still bounded rather than disabled: the blocking client has no inactivity
/// timeout, so a stalled upstream would otherwise pin this request's thread
/// forever. Generous enough that no real music video on a real link comes close.
const VIDEO_BODY_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Stream a progressive media file, forwarding the client's `Range`.
///
/// Separate from [`serve_segment`] because their shapes differ where it counts.
/// A segment is a few hundred KB read whole; a music video is tens of MB the
/// `<video>` element expects to seek within. `serve_segment` buffers the entire
/// body into a `Vec` and never forwards `Range`, which for a video means the
/// first frame waits on the last byte and the scrub bar does nothing.
///
/// Here the body is piped straight through and `Range` is passed both ways, so
/// the element gets its `206` and can seek. Everything else is [`serve_segment`]'s
/// reason for existing, unchanged: googlevideo needs the User-Agent of the client
/// that resolved the URL, and the CSP only lets the webview load from loopback.
///
/// **A `Range` header always goes upstream, invented if the element didn't send
/// one.** googlevideo paces a request that carries no `Range` to roughly the
/// bitrate of what was asked for — it assumes it is feeding a player watching in
/// real time. Measured on the audio path: 106s versus 0.56s for the same 3.4MB
/// body, from the header alone. A video arriving at 1× realtime is a video that
/// can never buffer ahead. See [`range_plan`] for why the reply is then rewritten.
fn serve_video(
    client: &reqwest::blocking::Client,
    upstream: &str,
    ua: Option<&str>,
    request: tiny_http::Request,
) {
    let range = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str().to_string());
    let plan = range_plan(range.as_deref());

    let mut req = client
        .get(upstream)
        .timeout(VIDEO_BODY_TIMEOUT)
        .header(reqwest::header::RANGE, plan.upstream_range.as_str());
    if let Some(ua) = ua {
        req = req.header(reqwest::header::USER_AGENT, ua);
    }

    let Ok(resp) = req.send() else {
        respond(request, 502, "text/plain", b"upstream fetch failed".to_vec(), None);
        return;
    };
    // Carry the headers a seekable element needs; without Content-Range a 206 is
    // meaningless to it.
    let pick = |name: reqwest::header::HeaderName| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let content_type =
        pick(reqwest::header::CONTENT_TYPE).unwrap_or_else(|| "video/mp4".to_string());
    let (status, content_range) = presented(
        resp.status().as_u16(),
        pick(reqwest::header::CONTENT_RANGE),
        plan.synthetic,
    );
    let len = resp.content_length();

    let mut response = Response::new(
        StatusCode(status),
        Vec::new(),
        resp,
        len.map(|n| n as usize),
        None,
    );
    for (name, value) in [
        ("Content-Type", content_type.as_str()),
        ("Access-Control-Allow-Origin", "*"),
        // Lets the element discover it can seek at all.
        ("Accept-Ranges", "bytes"),
        ("Cache-Control", "no-store"),
    ] {
        if let Ok(h) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response.add_header(h);
        }
    }
    if let Some(cr) = content_range.as_deref() {
        if let Ok(h) = Header::from_bytes(b"Content-Range".as_ref(), cr.as_bytes()) {
            response.add_header(h);
        }
    }
    let _ = request.respond(response);
}

/// What to ask googlevideo for, and whether we made it up.
struct RangePlan {
    /// The `Range` value to send upstream. Never absent — that is the point.
    upstream_range: String,
    /// True when the element asked for the whole body and we asked for a range
    /// anyway. The reply then has to be translated back on the way out.
    synthetic: bool,
}

/// Decide the upstream `Range` for a client request that may not have one.
///
/// `bytes=0-` asks for *no less than the whole body*, so inventing it changes
/// nothing about what is fetched — only about how fast googlevideo is willing to
/// send it. Boundedness is irrelevant; the presence of the header is everything.
fn range_plan(client_range: Option<&str>) -> RangePlan {
    match client_range {
        Some(r) => RangePlan { upstream_range: r.to_string(), synthetic: false },
        None => RangePlan { upstream_range: "bytes=0-".to_string(), synthetic: true },
    }
}

/// Translate the upstream reply back into one that answers the request the
/// client actually made.
///
/// A client that sent no `Range` is owed a `200` with no `Content-Range`. Handing
/// it the `206` our invented header earned would be answering a question it never
/// asked — and a `206` to a request without a `Range` is a response WebKit is
/// entitled to reject. The body is byte-identical either way, so only the framing
/// needs undoing.
fn presented(
    upstream_status: u16,
    content_range: Option<String>,
    synthetic: bool,
) -> (u16, Option<String>) {
    if synthetic && upstream_status == 206 {
        return (200, None);
    }
    (upstream_status, content_range)
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

    /// googlevideo paces a request carrying no `Range` to about the bitrate of
    /// what was asked for. Nothing in the *response* reveals that a body was
    /// paced, so the only place this can be guarded is here, on the way out.
    #[test]
    fn a_range_always_goes_upstream() {
        assert_eq!(range_plan(None).upstream_range, "bytes=0-");
        assert!(range_plan(None).synthetic);
    }

    /// The element's own range wins whenever it has one — it is seeking, and
    /// second-guessing where it wants to read from would break the seek.
    #[test]
    fn the_elements_own_range_is_forwarded_unchanged() {
        let plan = range_plan(Some("bytes=1048576-2097151"));
        assert_eq!(plan.upstream_range, "bytes=1048576-2097151");
        assert!(!plan.synthetic);
    }

    /// A `206` is only an answer to a question the client asked. Ours invented
    /// the question, so the client must still see the `200` it was owed.
    #[test]
    fn an_invented_range_does_not_leak_a_206_to_the_client() {
        let (status, cr) = presented(206, Some("bytes 0-99/100".into()), true);
        assert_eq!(status, 200);
        assert_eq!(cr, None, "a 200 carrying Content-Range is a contradiction");
    }

    /// When the element asked for a range, its `206` and `Content-Range` are the
    /// whole mechanism of seeking and must survive untouched.
    #[test]
    fn a_real_range_request_keeps_its_206_and_content_range() {
        let (status, cr) = presented(206, Some("bytes 10-19/100".into()), false);
        assert_eq!(status, 206);
        assert_eq!(cr.as_deref(), Some("bytes 10-19/100"));
    }

    /// Only `206` is ours to undo. A server that ignored the invented header and
    /// sent the whole body, or that failed, is already answering correctly.
    #[test]
    fn other_statuses_pass_through_whoever_asked() {
        assert_eq!(presented(200, None, true), (200, None));
        assert_eq!(presented(403, None, true), (403, None));
        assert_eq!(presented(416, None, false).0, 416);
    }

    /// The same claim as [`a_range_always_goes_upstream`], asked of the wire.
    ///
    /// Worth doing twice because nothing in a *response* reveals that a body was
    /// paced — a throttled fetch and a fast one are byte-identical, just minutes
    /// apart. A pure-function test says the plan is right; only a socket says the
    /// plan reached the socket. A refactor that dropped the header would leave
    /// every other test in this file green.
    #[test]
    fn the_proxy_puts_a_range_on_the_wire_when_the_element_did_not() {
        use std::io::{BufRead, BufReader, Write};

        let upstream = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();

        // Stand in for googlevideo: record what was asked, answer like it does.
        let seen = std::thread::spawn(move || {
            let (mut sock, _) = upstream.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut headers = Vec::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                headers.push(line.trim_end().to_string());
            }
            sock.write_all(
                b"HTTP/1.1 206 Partial Content\r\n\
                  Content-Type: video/mp4\r\n\
                  Content-Range: bytes 0-3/4\r\n\
                  Content-Length: 4\r\n\
                  Connection: close\r\n\r\nabcd",
            )
            .unwrap();
            sock.flush().unwrap();
            headers
        });

        let proxy = start().expect("proxy must start");
        let target = format!("http://127.0.0.1:{upstream_port}/videoplayback");
        let resp = reqwest::blocking::Client::new()
            .get(proxy.video_url(&target, None))
            .send()
            .expect("proxy must answer");

        // What the client is owed: it never asked for a range.
        assert_eq!(resp.status().as_u16(), 200);
        assert!(resp.headers().get(reqwest::header::CONTENT_RANGE).is_none());
        assert_eq!(resp.bytes().unwrap().as_ref(), b"abcd");

        // What googlevideo was actually asked — the whole point.
        let headers = seen.join().unwrap();
        let range = headers
            .iter()
            .find(|h| h.to_ascii_lowercase().starts_with("range:"))
            .expect("a request with no Range is one googlevideo will pace to ~1x realtime");
        assert!(
            range.to_ascii_lowercase().contains("bytes=0-"),
            "expected the whole body asked for as a range, got {range:?}"
        );
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
