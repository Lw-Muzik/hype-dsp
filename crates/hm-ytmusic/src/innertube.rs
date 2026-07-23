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

use crate::ytdlp::{self, StreamTarget};
use serde_json::Value;

// ---------------------------------------------------------------------------
// THE THING THAT ROTS: the ANDROID_VR client identity the `player` call
// presents. When the native hit-rate collapses, refresh these from yt-dlp's
// `_base_client` for android_vr (yt_dlp/extractor/youtube/_base.py) *before*
// suspecting SABR — a stale version here looks identical to SABR rollout.
const CLIENT_NAME: &str = "ANDROID_VR";
const CLIENT_VERSION: &str = "1.62.27";
/// Numeric id for the X-YouTube-Client-Name header (android_vr = 28).
const CLIENT_ID: &str = "28";
pub(crate) const ANDROID_VR_UA: &str =
    "com.google.android.apps.youtube.vr.oculus/1.62.27 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip";
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
#[allow(dead_code)] // consumed by Task 2's resolve_with_fallback native-first wiring
pub(crate) fn resolve_native(
    client: &reqwest::blocking::Client,
    video_id: &str,
) -> Result<StreamTarget, NativeMiss> {
    let body = serde_json::json!({
        "context": {
            "client": {
                "clientName": CLIENT_NAME,
                "clientVersion": CLIENT_VERSION,
                "androidSdkVersion": 32,
                "userAgent": ANDROID_VR_UA,
                "osName": "Android",
                "osVersion": "12L",
                "hl": "en",
                "gl": "US",
            }
        },
        "videoId": video_id,
        "cpn": cpn(),
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    let resp = client
        .post("https://youtubei.googleapis.com/youtubei/v1/player?prettyPrint=false")
        .header("User-Agent", ANDROID_VR_UA)
        .header("X-YouTube-Client-Name", CLIENT_ID)
        .header("X-YouTube-Client-Version", CLIENT_VERSION)
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(3))
        .json(&body)
        .send()
        .map_err(|e| NativeMiss::Http(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(NativeMiss::Http(format!("status {}", resp.status())));
    }
    let json: Value = resp.json().map_err(|_| NativeMiss::BadJson)?;
    parse_player_response(&json)
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
pub(crate) fn parse_player_response(json: &Value) -> Result<StreamTarget, NativeMiss> {
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
        headers: vec![("User-Agent".into(), ANDROID_VR_UA.into())],
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
        let t = parse_player_response(&player_ok(url)).expect("itag 140 with a url must resolve");
        assert_eq!(t.url, url);
        assert_eq!(t.ext, "m4a");
        assert_eq!(t.format_id, "140");
        assert_eq!(t.expires_at, Some(1_790_000_000));
        assert_eq!(t.abr_kbps, Some(130));
        // The media GET must present the same client the URL was minted for.
        assert_eq!(t.headers, vec![("User-Agent".to_string(), ANDROID_VR_UA.to_string())]);
    }

    /// The decoder pin: Opus must be unrepresentable — 140 or nothing.
    #[test]
    fn opus_alone_is_a_miss_never_a_target() {
        let mut v = player_ok("https://cdn/x");
        v["streamingData"]["adaptiveFormats"]
            .as_array_mut()
            .unwrap()
            .retain(|f| f["itag"] != 140);
        assert!(matches!(parse_player_response(&v), Err(NativeMiss::NoItag140)));
    }

    /// The SABR canary: a ciphered 140 means the exempt-client era is ending
    /// for this client — the log must say so distinctly.
    #[test]
    fn a_ciphered_format_is_its_own_miss() {
        let mut v = player_ok("unused");
        let f = &mut v["streamingData"]["adaptiveFormats"][1];
        f.as_object_mut().unwrap().remove("url");
        f["signatureCipher"] = serde_json::json!("s=abc&sp=sig&url=https%3A%2F%2F...");
        assert!(matches!(parse_player_response(&v), Err(NativeMiss::Ciphered)));
    }

    #[test]
    fn a_non_ok_playability_carries_its_status() {
        let mut v = player_ok("unused");
        v["playabilityStatus"] = serde_json::json!({ "status": "UNPLAYABLE", "reason": "made for kids" });
        match parse_player_response(&v) {
            Err(NativeMiss::Unplayable(s)) => assert_eq!(s, "UNPLAYABLE"),
            other => panic!("expected Unplayable, got {other:?}"),
        }
    }

    #[test]
    fn garbage_is_bad_json_not_a_panic() {
        assert!(matches!(
            parse_player_response(&serde_json::json!("not an object")),
            Err(NativeMiss::BadJson)
        ));
        assert!(matches!(
            parse_player_response(&serde_json::json!({})),
            Err(NativeMiss::BadJson)
        ));
    }

    /// A 140 with a url but no expire= still plays now — but must never enter
    /// the cache as immortal. parse_expiry returning None is the existing
    /// "immediately stale" posture; the target itself is still valid.
    #[test]
    fn a_url_without_expiry_resolves_with_none() {
        let t = parse_player_response(&player_ok("https://cdn/no-expiry")).unwrap();
        assert_eq!(t.expires_at, None);
    }
}
