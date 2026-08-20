//! Native InnerTube `player` resolution — the fast path past yt-dlp.
//!
//! One HTTPS POST replaces a ~5s Python process for the audio-stream resolve.
//! The ANDROID_VR client is the load-bearing choice: it needs no PO token and
//! returns plaintext urls (no signature cipher, no n-param JS) — it is
//! yt-dlp's own first-choice client, and the urls this app already streams
//! carry `c=ANDROID_VR`. Everything here MUST miss cleanly (never error the
//! caller): the yt-dlp fallback is the design's safety floor, and when
//! YouTube eventually moves this client to SABR the only symptom must be the
//! miss-tally in the log.
//!
//! **That floor gave way on 2026-08-18.** YouTube began proof-of-origin gating
//! ANDROID_VR **from client version 1.64**, and yt-dlp 2026.07.04 — the latest
//! release — pins `1.65.10`. So the fallback started returning urls that
//! resolve `OK`, serve a bounded slice, and 403 the open-ended GET a player
//! opens with: silence, not an error. The native path is no longer merely a
//! latency win over yt-dlp; while upstream is pinned above the wall it is the
//! only path here that returns a url which plays. See [`ANDROID_VR_VERSIONS`].

use crate::ytdlp::{self, StreamTarget};
use serde_json::Value;

// ---------------------------------------------------------------------------
// THE THING THAT ROTS: the ANDROID_VR client identities the `player` call
// presents. When the native hit-rate collapses, re-measure the table below
// *before* suspecting SABR — a stale version here looks identical to a SABR
// rollout, and copying yt-dlp's `_base_client` is no longer safe: as of
// 2026-08-18 upstream's pin is on the wrong side of the wall.
const CLIENT_NAME: &str = "ANDROID_VR";
/// Numeric id for the X-YouTube-Client-Name header (android_vr = 28).
const CLIENT_ID: &str = "28";

/// The ANDROID_VR client versions worth asking, in the order they are asked.
///
/// Measured 2026-08-18 on one session, one minute, one video, itag 140:
///
/// | clientVersion      | playabilityStatus | `Range: bytes=0-` |
/// |--------------------|-------------------|-------------------|
/// | 1.57.29 – 1.62.20  | OK (bot-checked sometimes) | **206**  |
/// | 1.63.24 / 1.63.29  | LOGIN_REQUIRED    | —                 |
/// | 1.64.16 … 1.68.24  | **OK**            | **403**           |
/// | 1.70.10            | OK                | no itag 140       |
///
/// There is a wall on each side. **From 1.64 every url is proof-of-origin
/// gated** — it resolves `OK` and then refuses the one open-ended GET a player
/// opens with, which reads to a user as a track that skips without playing.
/// Below 1.64 the urls are un-gated but the client is bot-checked, and *which*
/// version is refused moves between sessions. So no single version is
/// dependable and they are asked in turn: a different version is a genuinely
/// different request where a repeat of the refused one is not.
///
/// **Every version here has been measured serving an open-ended GET.** That is
/// the only probe that means anything — a gated url serves bounded ranges
/// inside its first mebibyte perfectly happily, which is exactly how this
/// shipped past a probe that read two bytes. Never add a version without that
/// measurement, and never add one at or above 1.64.
pub(crate) const ANDROID_VR_VERSIONS: &[&str] =
    &["1.62.20", "1.63.24", "1.61.48", "1.60.19"];

/// How long the whole rotation may run before the yt-dlp floor is the better
/// bet. This path exists to beat a ~5 s process; a rotation that outlives that
/// has defeated its own purpose, so it stops and lets the fallback take over.
const ROTATION_BUDGET: std::time::Duration = std::time::Duration::from_secs(4);

pub(crate) fn android_vr_ua(version: &str) -> String {
    format!(
        "com.google.android.apps.youtube.vr.oculus/{version} (Linux; U; \
         Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip"
    )
}

/// The `visitorData` this process has been issued, and the whole of what
/// separates "ANDROID_VR is bot-gated" from "ANDROID_VR works".
///
/// A `LOGIN_REQUIRED` response carries a usable one in its own
/// `responseContext`, so one is always obtainable — which is why the request
/// that is refused is never the last word. Held for the life of the process:
/// nothing about it is worth persisting.
static VISITOR_DATA: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn visitor_data() -> Option<String> {
    VISITOR_DATA.lock().ok().and_then(|held| held.clone())
}

fn set_visitor_data(value: Option<String>) {
    if let Ok(mut slot) = VISITOR_DATA.lock() {
        *slot = value;
    }
}

/// Captured from *any* response — refusals carry one too.
fn harvest_visitor_data(json: &Value) {
    if let Some(v) = json
        .pointer("/responseContext/visitorData")
        .and_then(Value::as_str)
    {
        set_visitor_data(Some(v.to_string()));
    }
}
// ---------------------------------------------------------------------------

/// Every way the native resolve can come up short. Never surfaced to the
/// caller as an error — only ever a reason the yt-dlp fallback took over, and
/// a line in the miss tally.
#[derive(Debug)]
pub(crate) enum NativeMiss {
    /// Transport failure or a non-2xx status — includes timeouts.
    Http(String),
    /// The response wasn't the JSON shape expected at all.
    BadJson,
    /// `playabilityStatus.status != "OK"` — carries the status string.
    Unplayable(String),
    /// No itag 140 in `adaptiveFormats` at all.
    NoItag140,
    /// itag 140 exists but hides its url behind `signatureCipher` — the SABR
    /// canary: this client losing its plaintext-url exemption.
    Ciphered,
}

impl std::fmt::Display for NativeMiss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(m) => write!(f, "http: {m}"),
            Self::BadJson => write!(f, "bad json"),
            Self::Unplayable(status) => write!(f, "unplayable: {status}"),
            Self::NoItag140 => write!(f, "no itag 140"),
            Self::Ciphered => write!(f, "ciphered"),
        }
    }
}

/// One `player` POST → a validated-shape (not yet probed) StreamTarget.
///
/// Every fallible step here maps its error into a [`NativeMiss`] rather than
/// propagating — this function has no error path that isn't one of the enum's
/// variants, by construction.
pub(crate) fn resolve_native(
    client: &reqwest::blocking::Client,
    video_id: &str,
) -> Result<StreamTarget, NativeMiss> {
    let started = std::time::Instant::now();
    let mut last = NativeMiss::Http("no identity tried".to_string());
    for version in ANDROID_VR_VERSIONS {
        match ask_presenting(client, video_id, version) {
            Ok(target) => return Ok(target),
            Err(miss) => last = miss,
        }
        // The floor is 5 seconds away; spending longer than that here to avoid
        // it is a loss even when the next identity would have worked.
        if started.elapsed() >= ROTATION_BUDGET {
            break;
        }
    }
    Err(last)
}

/// One client identity, asked with the token held and — if it is refused —
/// once more with a genuinely different one.
///
/// A refusal always carries a `visitorData`, so the second ask is either "this
/// identity, on the session that refusal just minted" or, when nothing new came
/// back, a cold one: the only remaining way to make the request differ, and it
/// mints a fresh session out of its own refusal for whoever asks next.
///
/// Only `Unplayable` is retried. A missing itag 140 or a ciphered url is a fact
/// about this identity, not about the session, and asking again cannot change
/// it — the next *version* might, which is the loop above's job.
fn ask_presenting(
    client: &reqwest::blocking::Client,
    video_id: &str,
    version: &str,
) -> Result<StreamTarget, NativeMiss> {
    let presented = visitor_data();
    let json = player_once(client, video_id, version, presented.as_deref())?;
    let miss = match parse_player_response(&json, version) {
        Ok(target) => return Ok(target),
        Err(miss) => miss,
    };
    if !matches!(miss, NativeMiss::Unplayable(_)) {
        return Err(miss);
    }

    let learned = visitor_data();
    if learned.is_some() && learned != presented {
        let json = player_once(client, video_id, version, learned.as_deref())?;
        return parse_player_response(&json, version);
    }
    // Nothing new came back and we presented nothing, so there is no second
    // request to make that differs from the first.
    if presented.is_none() {
        return Err(miss);
    }
    set_visitor_data(None);
    let json = player_once(client, video_id, version, None)?;
    parse_player_response(&json, version)
}

/// One `player` POST, exactly as presented to YouTube.
fn player_once(
    client: &reqwest::blocking::Client,
    video_id: &str,
    version: &str,
    visitor: Option<&str>,
) -> Result<Value, NativeMiss> {
    let ua = android_vr_ua(version);
    let mut identity = serde_json::json!({
        "clientName": CLIENT_NAME,
        "clientVersion": version,
        "androidSdkVersion": 32,
        "userAgent": ua,
        "deviceMake": "Oculus",
        "deviceModel": "Quest 3",
        "osName": "Android",
        "osVersion": "12L",
        "hl": "en",
        "gl": "US",
    });
    if let Some(v) = visitor {
        identity["visitorData"] = Value::String(v.to_string());
    }
    let body = serde_json::json!({
        "context": { "client": identity },
        "videoId": video_id,
        "cpn": cpn(),
        "contentCheckOk": true,
        "racyCheckOk": true,
    });

    let mut req = client
        .post("https://youtubei.googleapis.com/youtubei/v1/player?prettyPrint=false")
        .header("User-Agent", ua.as_str())
        .header("X-YouTube-Client-Name", CLIENT_ID)
        .header("X-YouTube-Client-Version", version)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(3));
    if let Some(v) = visitor {
        req = req.header("X-Goog-Visitor-Id", v);
    }
    let resp = req
        .json(&body)
        .send()
        .map_err(|e| NativeMiss::Http(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(NativeMiss::Http(format!("status {}", resp.status())));
    }
    let json: Value = resp.json().map_err(|_| NativeMiss::BadJson)?;
    harvest_visitor_data(&json);
    Ok(json)
}

/// A fresh client-playback nonce: 16 URL-safe chars. Uniqueness matters,
/// unpredictability doesn't — clock nanos are plenty, and pulling in a real
/// RNG for a value nothing security-sensitive depends on isn't worth a dep.
fn cpn() -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut out = String::with_capacity(16);
    for _ in 0..16 {
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
        n >>= 6;
    }
    out
}

/// Reads a `player` response into a [`StreamTarget`], or the specific way it
/// fell short. Pure and total: no network, no panics — every branch either
/// returns `Ok` or one of [`NativeMiss`]'s variants.
pub(crate) fn parse_player_response(
    json: &Value,
    version: &str,
) -> Result<StreamTarget, NativeMiss> {
    let status = json
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str)
        .ok_or(NativeMiss::BadJson)?;
    if status != "OK" {
        return Err(NativeMiss::Unplayable(status.to_string()));
    }
    let formats = json
        .pointer("/streamingData/adaptiveFormats")
        .and_then(Value::as_array)
        .ok_or(NativeMiss::BadJson)?;
    let m4a = formats
        .iter()
        .find(|f| f.pointer("/itag").and_then(Value::as_u64) == Some(140))
        .ok_or(NativeMiss::NoItag140)?;
    let url = match m4a.pointer("/url").and_then(Value::as_str) {
        Some(u) => u,
        // A 140 that exists but hides its url behind a cipher is the SABR
        // canary — distinct from "no 140 at all" so the log shows the drift.
        None => return Err(NativeMiss::Ciphered),
    };
    Ok(StreamTarget {
        url: url.to_string(),
        headers: vec![("User-Agent".into(), android_vr_ua(version))],
        ext: "m4a".into(),
        format_id: "140".into(),
        abr_kbps: m4a
            .pointer("/bitrate")
            .and_then(Value::as_u64)
            .and_then(|b| u32::try_from(b / 1000).ok()),
        expires_at: ytdlp::parse_expiry(url),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player_ok(url: &str) -> serde_json::Value {
        serde_json::json!({
            "playabilityStatus": { "status": "OK" },
            "streamingData": {
                "adaptiveFormats": [
                    { "itag": 251, "mimeType": "audio/webm; codecs=\"opus\"", "url": "https://cdn/opus" },
                    { "itag": 140, "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"", "bitrate": 130_000, "url": url },
                    { "itag": 136, "mimeType": "video/mp4", "url": "https://cdn/video" }
                ]
            }
        })
    }

    #[test]
    fn a_healthy_response_builds_the_exact_target() {
        let url = "https://rr3.googlevideo.com/videoplayback?expire=1790000000&c=ANDROID_VR&itag=140";
        let t = parse_player_response(&player_ok(url), "1.62.20").expect("itag 140 with a url must resolve");
        assert_eq!(t.url, url);
        assert_eq!(t.ext, "m4a");
        assert_eq!(t.format_id, "140");
        assert_eq!(t.expires_at, Some(1_790_000_000));
        assert_eq!(t.abr_kbps, Some(130));
        // The media GET must present the same client the URL was minted for.
        assert_eq!(
            t.headers,
            vec![("User-Agent".to_string(), android_vr_ua("1.62.20"))]
        );
    }

    /// The decoder pin: Opus must be unrepresentable — 140 or nothing.
    #[test]
    fn opus_alone_is_a_miss_never_a_target() {
        let mut v = player_ok("https://cdn/x");
        v["streamingData"]["adaptiveFormats"]
            .as_array_mut()
            .unwrap()
            .retain(|f| f["itag"] != 140);
        assert!(matches!(parse_player_response(&v, "1.62.20"), Err(NativeMiss::NoItag140)));
    }

    /// The SABR canary: a ciphered 140 means the exempt-client era is ending
    /// for this client — the log must say so distinctly.
    #[test]
    fn a_ciphered_format_is_its_own_miss() {
        let mut v = player_ok("unused");
        let f = &mut v["streamingData"]["adaptiveFormats"][1];
        f.as_object_mut().unwrap().remove("url");
        f["signatureCipher"] = serde_json::json!("s=abc&sp=sig&url=https%3A%2F%2F...");
        assert!(matches!(parse_player_response(&v, "1.62.20"), Err(NativeMiss::Ciphered)));
    }

    #[test]
    fn a_non_ok_playability_carries_its_status() {
        let mut v = player_ok("unused");
        v["playabilityStatus"] = serde_json::json!({ "status": "UNPLAYABLE", "reason": "made for kids" });
        match parse_player_response(&v, "1.62.20") {
            Err(NativeMiss::Unplayable(s)) => assert_eq!(s, "UNPLAYABLE"),
            other => panic!("expected Unplayable, got {other:?}"),
        }
    }

    /// `1.64.16` -> 164016, so versions compare as numbers rather than strings
    /// (where "1.9" would sort above "1.64").
    fn ordinal(version: &str) -> u32 {
        let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap());
        parts.next().unwrap() * 1_000_000
            + parts.next().unwrap() * 1_000
            + parts.next().unwrap()
    }

    /// The 2026-08-18 outage, and the one thing that must never regress.
    ///
    /// From 1.64 upward YouTube hands ANDROID_VR a proof-of-origin gated url:
    /// it resolves `OK`, it serves a bounded slice, and it 403s the single
    /// open-ended GET a player opens with. Nothing before playback tells one
    /// apart from a good url — which is why `probe_ok` reading two bytes could
    /// not catch it, and why this assertion exists instead.
    #[test]
    fn every_identity_is_below_the_gate_at_1_64() {
        for version in ANDROID_VR_VERSIONS {
            assert!(
                ordinal(version) < ordinal("1.64.0"),
                "{version} resolves OK and then plays nothing at all"
            );
        }
    }

    /// One version is a single point of failure against a bot check whose
    /// verdict moves between sessions.
    #[test]
    fn there_is_more_than_one_identity_to_fall_back_on() {
        assert!(ANDROID_VR_VERSIONS.len() > 1);
    }

    #[test]
    fn the_user_agent_names_the_version_that_minted_the_url() {
        let t = parse_player_response(&player_ok("https://cdn/x"), "1.61.48")
            .expect("itag 140 with a url must resolve");
        assert_eq!(
            t.headers,
            vec![(
                "User-Agent".to_string(),
                "com.google.android.apps.youtube.vr.oculus/1.61.48 (Linux; U; \
                 Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip"
                    .to_string()
            )],
            "googlevideo checks the UA against the resolving client when it \
             chooses to; a mismatched one is a way to 403 for free"
        );
    }

    #[test]
    fn garbage_is_bad_json_not_a_panic() {
        assert!(matches!(
            parse_player_response(&serde_json::json!("not an object"), "1.62.20"),
            Err(NativeMiss::BadJson)
        ));
        assert!(matches!(
            parse_player_response(&serde_json::json!({}), "1.62.20"),
            Err(NativeMiss::BadJson)
        ));
    }

    /// A 140 with a url but no expire= still plays now — but must never enter
    /// the cache as immortal. parse_expiry returning None is the existing
    /// "immediately stale" posture; the target itself is still valid.
    #[test]
    fn a_url_without_expiry_resolves_with_none() {
        let t = parse_player_response(&player_ok("https://cdn/no-expiry"), "1.62.20").unwrap();
        assert_eq!(t.expires_at, None);
    }
}
