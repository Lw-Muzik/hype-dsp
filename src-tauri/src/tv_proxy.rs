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

/// Serve a googlevideo video-only rendition to the `<video>` element, fetching
/// it in bounded chunks.
///
/// The chunking is not an optimisation — it is the only thing that works.
/// googlevideo **403s an open-ended or oversized `Range`** on these urls (itag
/// 136 via the ANDROID_VR client): measured, `bytes=0-` and any range past ~5 MB
/// are rejected outright, while a bounded ≤~3 MB range is served in a fraction of
/// a second. A proxy that forwards the element's `bytes=0-`, or invents one, gets
/// a 403 — and the element limps to a picture only after a minute-plus of retry
/// (the "video takes 1:36 to show up" bug). So the proxy never asks googlevideo
/// for more than [`VIDEO_CHUNK`] at once and stitches the chunks into one
/// continuous body the element sees as an ordinary seekable file.
///
/// The moov sits at the front of these renditions, so the first chunk carries
/// everything needed to start — the first frame lands in about a second.
///
/// The element's `Range` (or its absence) only decides the *framing*: no range →
/// `200` over the whole file; `bytes=A-B` / `bytes=A-` → `206` from `A`. The UA
/// still goes upstream, since googlevideo checks it against the resolving client.
fn serve_video(
    client: &reqwest::blocking::Client,
    upstream: &str,
    ua: Option<&str>,
    request: tiny_http::Request,
) {
    let Some(total) = video_total_len(client, upstream, ua) else {
        respond(request, 502, "text/plain", b"upstream length unknown".to_vec(), None);
        return;
    };
    let range = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str().to_string());
    let framing = video_framing(range.as_deref(), total);

    let reader = ChunkedVideo {
        client: client.clone(),
        url: upstream.to_string(),
        ua: ua.map(str::to_string),
        next: framing.start,
        end: framing.end,
        buf: Vec::new(),
        buf_pos: 0,
    };
    let content_len = framing.end - framing.start + 1;
    let mut response = Response::new(
        StatusCode(framing.status),
        Vec::new(),
        reader,
        Some(content_len as usize),
        None,
    );
    for (name, value) in [
        ("Content-Type", "video/mp4"),
        ("Access-Control-Allow-Origin", "*"),
        ("Accept-Ranges", "bytes"),
        ("Cache-Control", "no-store"),
    ] {
        if let Ok(h) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response.add_header(h);
        }
    }
    if let Some(cr) = framing.content_range.as_deref() {
        if let Ok(h) = Header::from_bytes(b"Content-Range".as_ref(), cr.as_bytes()) {
            response.add_header(h);
        }
    }
    let _ = request.respond(response);
}

/// The largest range googlevideo will serve in one request, with margin.
///
/// Measured against a real itag-136 url: ≤3 MB is served fast, ≥8 MB is a hard
/// 403, 5 MB is served but already paced. 1 MB sits well inside the safe band and
/// keeps the first chunk — the one carrying the moov — as small and fast as it
/// can be, which is what makes the first frame appear at once.
const VIDEO_CHUNK: u64 = 1 << 20;

/// How the element's `Range` maps onto the response we send back.
struct VideoFraming {
    status: u16,
    start: u64,
    /// Inclusive last byte.
    end: u64,
    content_range: Option<String>,
}

fn video_framing(client_range: Option<&str>, total: u64) -> VideoFraming {
    let last = total.saturating_sub(1);
    match parse_byte_range(client_range) {
        // No Range: the element is owed a 200 over the whole file.
        None => VideoFraming { status: 200, start: 0, end: last, content_range: None },
        // A Range: a 206 from `start`, clamped to what exists.
        Some((start, end_opt)) => {
            let start = start.min(last);
            let end = end_opt.unwrap_or(last).min(last);
            VideoFraming {
                status: 206,
                start,
                end,
                content_range: Some(format!("bytes {start}-{end}/{total}")),
            }
        }
    }
}

/// Parse a `Range: bytes=A-B` / `bytes=A-` header into `(start, end_inclusive?)`.
/// Only the single-range `bytes=` form a `<video>` element sends; anything else
/// (multi-range, a suffix `bytes=-N`, garbage) is treated as no range.
fn parse_byte_range(header: Option<&str>) -> Option<(u64, Option<u64>)> {
    let spec = header?.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None; // multi-range: can't be one contiguous body
    }
    let (a, b) = spec.split_once('-')?;
    let start: u64 = a.trim().parse().ok()?;
    let end = match b.trim() {
        "" => None,
        s => Some(s.parse().ok()?),
    };
    Some((start, end))
}

/// The full length of the rendition, for framing and clamping.
///
/// googlevideo stamps `clen=` on every url, so it's read from there without a
/// request. The `bytes=0-0` probe is only a fallback for a url that somehow lacks
/// it — one tiny request that reads the total out of `Content-Range`.
fn video_total_len(
    client: &reqwest::blocking::Client,
    url: &str,
    ua: Option<&str>,
) -> Option<u64> {
    if let Some(n) = parse_clen(url) {
        return Some(n);
    }
    let mut req = client
        .get(url)
        .timeout(VIDEO_BODY_TIMEOUT)
        .header(reqwest::header::RANGE, "bytes=0-0");
    if let Some(ua) = ua {
        req = req.header(reqwest::header::USER_AGENT, ua);
    }
    let resp = req.send().ok()?;
    let cr = resp.headers().get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    total_from_content_range(cr)
}

/// Read `clen=<n>` out of a googlevideo url's query.
fn parse_clen(url: &str) -> Option<u64> {
    url.split(['?', '&'])
        .find_map(|kv| kv.strip_prefix("clen="))
        .and_then(|v| v.parse().ok())
}

/// The total from a `Content-Range: bytes A-B/TOTAL` header.
fn total_from_content_range(cr: &str) -> Option<u64> {
    cr.rsplit('/').next()?.trim().parse().ok()
}

/// The next upstream chunk `(lo, hi_inclusive)` to fetch, or `None` at the end.
fn next_chunk(next: u64, end: u64, chunk: u64) -> Option<(u64, u64)> {
    if next > end {
        return None;
    }
    Some((next, (next + chunk - 1).min(end)))
}

/// A googlevideo body streamed to the element as bounded chunks.
///
/// Each `read` that empties the buffer fetches the next [`VIDEO_CHUNK`] range —
/// bounded, because open-ended or oversized is a 403 here. One chunk (~1 MB) is
/// resident at a time, so a 24 MB video never costs more than that in RAM.
struct ChunkedVideo {
    client: reqwest::blocking::Client,
    url: String,
    ua: Option<String>,
    next: u64,
    end: u64,
    buf: Vec<u8>,
    buf_pos: usize,
}

impl std::io::Read for ChunkedVideo {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.buf_pos >= self.buf.len() {
            let Some((lo, hi)) = next_chunk(self.next, self.end, VIDEO_CHUNK) else {
                return Ok(0); // whole requested span delivered
            };
            let mut req = self
                .client
                .get(&self.url)
                .timeout(VIDEO_BODY_TIMEOUT)
                .header(reqwest::header::RANGE, format!("bytes={lo}-{hi}"));
            if let Some(ua) = &self.ua {
                req = req.header(reqwest::header::USER_AGENT, ua);
            }
            let resp = req.send().map_err(std::io::Error::other)?;
            if !resp.status().is_success() {
                return Err(std::io::Error::other(format!(
                    "upstream chunk {lo}-{hi} -> {}",
                    resp.status()
                )));
            }
            self.buf = resp
                .bytes()
                .map_err(std::io::Error::other)?
                .to_vec();
            self.buf_pos = 0;
            self.next = hi + 1;
            if self.buf.is_empty() {
                return Ok(0);
            }
        }
        let n = (self.buf.len() - self.buf_pos).min(out.len());
        out[..n].copy_from_slice(&self.buf[self.buf_pos..self.buf_pos + n]);
        self.buf_pos += n;
        Ok(n)
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
    fn parses_a_bounded_and_open_ended_range() {
        assert_eq!(parse_byte_range(Some("bytes=100-199")), Some((100, Some(199))));
        assert_eq!(parse_byte_range(Some("bytes=100-")), Some((100, None)));
        assert_eq!(parse_byte_range(Some(" bytes=0-0 ")), Some((0, Some(0))));
    }

    /// Anything that isn't a single `bytes=A-B` range is treated as "no range" —
    /// the element never sends those, and we can't frame them as one body.
    #[test]
    fn non_single_ranges_are_treated_as_absent() {
        assert_eq!(parse_byte_range(None), None);
        assert_eq!(parse_byte_range(Some("bytes=0-1,5-9")), None); // multi-range
        assert_eq!(parse_byte_range(Some("bytes=-500")), None); // suffix form
        assert_eq!(parse_byte_range(Some("seconds=0-1")), None); // wrong unit
        assert_eq!(parse_byte_range(Some("garbage")), None);
    }

    /// No range from the element is a 200 over the whole file; a range is a 206
    /// framed against the real total. This framing is independent of how the
    /// body is then fetched (chunked), which is the point of testing it apart.
    #[test]
    fn framing_maps_the_element_range_onto_the_response() {
        let f = video_framing(None, 1000);
        assert_eq!((f.status, f.start, f.end), (200, 0, 999));
        assert!(f.content_range.is_none(), "a 200 must carry no Content-Range");

        let f = video_framing(Some("bytes=200-399"), 1000);
        assert_eq!((f.status, f.start, f.end), (206, 200, 399));
        assert_eq!(f.content_range.as_deref(), Some("bytes 200-399/1000"));

        // Open-ended range runs to the last byte.
        let f = video_framing(Some("bytes=200-"), 1000);
        assert_eq!((f.status, f.start, f.end), (206, 200, 999));
        assert_eq!(f.content_range.as_deref(), Some("bytes 200-999/1000"));
    }

    /// A range past the end must clamp, not overrun — else the Content-Length we
    /// promise and the bytes we deliver disagree and the element hangs.
    #[test]
    fn framing_clamps_a_range_past_the_end() {
        let f = video_framing(Some("bytes=900-5000"), 1000);
        assert_eq!((f.start, f.end), (900, 999));
        assert_eq!(f.content_range.as_deref(), Some("bytes 900-999/1000"));
    }

    #[test]
    fn reads_clen_from_the_googlevideo_url() {
        assert_eq!(parse_clen("https://gv/vp?itag=136&clen=26174413&dur=252"), Some(26174413));
        assert_eq!(parse_clen("https://gv/vp?clen=42"), Some(42));
        assert_eq!(parse_clen("https://gv/vp?itag=136"), None);
    }

    #[test]
    fn reads_total_from_a_content_range() {
        assert_eq!(total_from_content_range("bytes 0-0/26174413"), Some(26174413));
        assert_eq!(total_from_content_range("bytes 100-199/1000"), Some(1000));
        assert_eq!(total_from_content_range("nonsense"), None);
    }

    /// The chunk walk covers `[next, end]` exactly, in bounded steps, and stops.
    #[test]
    fn chunks_cover_the_span_in_bounded_steps() {
        assert_eq!(next_chunk(0, 9, 4), Some((0, 3)));
        assert_eq!(next_chunk(4, 9, 4), Some((4, 7)));
        assert_eq!(next_chunk(8, 9, 4), Some((8, 9)), "last chunk is short, not overrun");
        assert_eq!(next_chunk(10, 9, 4), None, "past the end: done");
        // A single request never exceeds the chunk size — that is the 403 guard.
        for start in [0u64, 100, 1_000_000] {
            let (lo, hi) = next_chunk(start, u64::MAX, VIDEO_CHUNK).unwrap();
            assert!(hi - lo + 1 <= VIDEO_CHUNK, "chunk must stay within the safe range size");
        }
    }

    /// End to end against a scripted googlevideo that behaves like the real one:
    /// it **403s an open-ended or oversized range** and serves only bounded
    /// chunks. The proxy must still deliver the whole body — proof it chunks,
    /// which a response alone (identical whether chunked or not) cannot show. A
    /// regression to a single unbounded fetch fails here and nowhere else.
    #[test]
    fn the_proxy_streams_a_whole_video_in_bounded_chunks() {
        use std::io::{BufRead, BufReader, Read, Write};

        // 3.5 chunks of body, so the walk takes several requests and a short tail.
        let total: usize = (VIDEO_CHUNK as usize) * 3 + 512;
        let body: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
        let body_for_server = body.clone();

        let upstream = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();

        let server = std::thread::spawn(move || {
            let mut max_seen: u64 = 0;
            // The proxy fetches clen from the url, so there's no probe — one
            // connection per chunk (Connection: close each time).
            loop {
                let Ok((mut sock, _)) = upstream.accept() else { break };
                let mut reader = BufReader::new(sock.try_clone().unwrap());
                let mut range = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    if line.to_ascii_lowercase().starts_with("range:") {
                        range = line.trim().to_string();
                    }
                }
                // Parse "Range: bytes=lo-hi".
                let spec = range.split_once("bytes=").map(|(_, r)| r.trim().to_string()).unwrap_or_default();
                let (lo, hi) = spec.split_once('-').unwrap_or(("", ""));
                let lo: u64 = lo.trim().parse().unwrap_or(0);
                let hi_opt: Option<u64> = hi.trim().parse().ok();

                // Reject exactly what real googlevideo rejects: open-ended, or a
                // span bigger than one chunk.
                let too_big = match hi_opt {
                    None => true,
                    Some(hi) => hi.saturating_sub(lo) + 1 > VIDEO_CHUNK,
                };
                if too_big {
                    sock.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    sock.flush().unwrap();
                    continue;
                }
                let hi = hi_opt.unwrap().min(total as u64 - 1);
                max_seen = max_seen.max(hi - lo + 1);
                let slice = &body_for_server[lo as usize..=hi as usize];
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nContent-Range: bytes {lo}-{hi}/{total}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    slice.len()
                );
                sock.write_all(header.as_bytes()).unwrap();
                sock.write_all(slice).unwrap();
                sock.flush().unwrap();
                if hi as usize + 1 >= total {
                    return max_seen;
                }
            }
            max_seen
        });

        let proxy = start().expect("proxy starts");
        let target = format!("http://127.0.0.1:{upstream_port}/videoplayback?clen={total}");
        let resp = reqwest::blocking::Client::new()
            .get(proxy.video_url(&target, None))
            .send()
            .expect("proxy answers");

        assert_eq!(resp.status().as_u16(), 200, "no element range ⇒ 200 over the whole file");
        let mut got = Vec::new();
        resp.take(total as u64 + 1024).read_to_end(&mut got).unwrap();
        assert_eq!(got.len(), total, "the proxy must deliver every byte");
        assert_eq!(got, body, "and the bytes must be the file, in order");

        let max_chunk = server.join().unwrap();
        assert!(
            max_chunk <= VIDEO_CHUNK,
            "the proxy asked for {max_chunk} bytes at once — googlevideo would 403 that"
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
