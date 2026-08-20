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
//! opens with: silence, not an error.
//!
//! **Then the other wall closed on 2026-08-20** and playback stopped outright,
//! with every track answering *"Sign in to confirm you're not a bot"*. Below
//! 1.64 ANDROID_VR is now bot-checked *wholesale* rather than occasionally, so
//! the version rotation built on 08-18 has nothing left to rotate to. The
//! client is refused on one side of the wall and gated on the other, and there
//! is no version of it that plays. VISIONOS replaces it — see
//! [`ClientIdentity`].

use crate::ytdlp::{self, StreamTarget};
use serde_json::Value;

// ---------------------------------------------------------------------------
// THE THING THAT ROTS: the client identities the `player` call presents. When
// the native hit-rate collapses, re-measure the table below *before*
// suspecting SABR — a stale identity here looks identical to a SABR rollout,
// and copying yt-dlp's `_base_client` wholesale is not safe either: on
// 2026-08-18 upstream's ANDROID_VR pin was on the wrong side of the wall.

/// One client identity, as presented to InnerTube.
///
/// Measured 2026-08-20, when playback stopped everywhere:
///
/// | client                | playabilityStatus  | `Range: bytes=0-`  |
/// |-----------------------|--------------------|--------------------|
/// | ANDROID_VR <  1.64    | **LOGIN_REQUIRED** | —                  |
/// | ANDROID_VR >= 1.64    | OK                 | **403**            |
/// | IOS / ANDROID         | OK                 | **403**            |
/// | TVHTML5 / *_MUSIC     | **LOGIN_REQUIRED** | —                  |
/// | **VISIONOS**          | **OK**             | **206, whole file**|
///
/// The gate on the `OK` rows is on the **byte offset**, exactly 1048576:
/// `bytes=0-1048575` serves and `bytes=1048576-` is 403. That is YouTube's
/// PO-token "cold start" allowance, so no proxy and no smaller range walks
/// around it — only a token or a client that does not need one.
///
/// VISIONOS is the one client left answering `OK` with a **plaintext** itag
/// 140 (no `signatureCipher`, so no n-param JS to solve) whose url serves a
/// whole file. Verified 5/5 on fresh search results, and corroborated
/// upstream: yt-dlp 2026.08.19 moved its own defaults to `('visionos', 'web')`
/// and marked `android_vr` PO-token-required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientIdentity {
    /// Apple Vision Pro. A single published identity rather than an app with a
    /// rolling version, so there is no version list to walk: when it is
    /// refused the second axis is the `visitorData`.
    VisionOs,
    /// Kept behind VISIONOS rather than deleted. Every version here is below
    /// 1.64, so it fails **loudly** — a refusal, not a gated url that plays a
    /// megabyte and stops — which costs one clean miss and leaves a second
    /// axis if VISIONOS is ever refused. Never add one at or above 1.64.
    AndroidVr(&'static str),
}

/// The identities worth asking, in the order they are asked.
pub(crate) const IDENTITIES: &[ClientIdentity] = &[
    ClientIdentity::VisionOs,
    ClientIdentity::AndroidVr("1.62.20"),
    ClientIdentity::AndroidVr("1.63.24"),
    ClientIdentity::AndroidVr("1.61.48"),
    ClientIdentity::AndroidVr("1.60.19"),
];

const VISIONOS_VERSION: &str = "1.02";
const VISIONOS_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) \
     AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

impl ClientIdentity {
    /// `clientName` in the InnerTube context.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::VisionOs => "VISIONOS",
            Self::AndroidVr(_) => "ANDROID_VR",
        }
    }

    /// The numeric id for the `X-YouTube-Client-Name` header.
    pub(crate) fn id(&self) -> &'static str {
        match self {
            Self::VisionOs => "101",
            Self::AndroidVr(_) => "28",
        }
    }

    pub(crate) fn version(&self) -> &'static str {
        match self {
            Self::VisionOs => VISIONOS_VERSION,
            Self::AndroidVr(v) => v,
        }
    }

    /// The UA presented on the `player` call **and** on the media GET:
    /// googlevideo checks it against the resolving client when it chooses to,
    /// so a mismatched one is a way to 403 for free.
    pub(crate) fn user_agent(&self) -> String {
        match self {
            Self::VisionOs => VISIONOS_UA.to_string(),
            Self::AndroidVr(v) => android_vr_ua(v),
        }
    }

    /// The `context.client` object. The device fields are not decoration — an
    /// identity YouTube cannot place is answered exactly like a bot.
    fn context(&self) -> Value {
        match self {
            Self::VisionOs => serde_json::json!({
                "clientName": self.name(),
                "clientVersion": VISIONOS_VERSION,
                "userAgent": VISIONOS_UA,
                "deviceMake": "Apple",
                "deviceModel": "RealityDevice17,1",
                "osName": "visionOS",
                "osVersion": "26.5.23O471",
                "hl": "en",
                "gl": "US",
            }),
            Self::AndroidVr(v) => serde_json::json!({
                "clientName": self.name(),
                "clientVersion": v,
                "androidSdkVersion": 32,
                "userAgent": android_vr_ua(v),
                "deviceMake": "Oculus",
                "deviceModel": "Quest 3",
                "osName": "Android",
                "osVersion": "12L",
                "hl": "en",
                "gl": "US",
            }),
        }
    }
}

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
    for identity in IDENTITIES {
        match ask_presenting(client, video_id, *identity) {
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
    identity: ClientIdentity,
) -> Result<StreamTarget, NativeMiss> {
    let presented = visitor_data();
    let json = player_once(client, video_id, identity, presented.as_deref())?;
    let miss = match parse_player_response(&json, identity) {
        Ok(target) => return Ok(target),
        Err(miss) => miss,
    };
    if !matches!(miss, NativeMiss::Unplayable(_)) {
        return Err(miss);
    }

    let learned = visitor_data();
    if learned.is_some() && learned != presented {
        let json = player_once(client, video_id, identity, learned.as_deref())?;
        return parse_player_response(&json, identity);
    }
    // Nothing new came back and we presented nothing, so there is no second
    // request to make that differs from the first.
    if presented.is_none() {
        return Err(miss);
    }
    set_visitor_data(None);
    let json = player_once(client, video_id, identity, None)?;
    parse_player_response(&json, identity)
}

/// One `player` POST, exactly as presented to YouTube.
fn player_once(
    client: &reqwest::blocking::Client,
    video_id: &str,
    identity: ClientIdentity,
    visitor: Option<&str>,
) -> Result<Value, NativeMiss> {
    let ua = identity.user_agent();
    let mut context = identity.context();
    if let Some(v) = visitor {
        context["visitorData"] = Value::String(v.to_string());
    }
    let body = serde_json::json!({
        "context": { "client": context },
        "videoId": video_id,
        "cpn": cpn(),
        "contentCheckOk": true,
        "racyCheckOk": true,
    });

    let mut req = client
        .post("https://youtubei.googleapis.com/youtubei/v1/player?prettyPrint=false")
        .header("User-Agent", ua.as_str())
        .header("X-YouTube-Client-Name", identity.id())
        .header("X-YouTube-Client-Version", identity.version())
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
    identity: ClientIdentity,
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
        headers: vec![("User-Agent".into(), identity.user_agent())],
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
        let t = parse_player_response(&player_ok(url), ClientIdentity::AndroidVr("1.62.20")).expect("itag 140 with a url must resolve");
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
        assert!(matches!(parse_player_response(&v, ClientIdentity::AndroidVr("1.62.20")), Err(NativeMiss::NoItag140)));
    }

    /// The SABR canary: a ciphered 140 means the exempt-client era is ending
    /// for this client — the log must say so distinctly.
    #[test]
    fn a_ciphered_format_is_its_own_miss() {
        let mut v = player_ok("unused");
        let f = &mut v["streamingData"]["adaptiveFormats"][1];
        f.as_object_mut().unwrap().remove("url");
        f["signatureCipher"] = serde_json::json!("s=abc&sp=sig&url=https%3A%2F%2F...");
        assert!(matches!(parse_player_response(&v, ClientIdentity::AndroidVr("1.62.20")), Err(NativeMiss::Ciphered)));
    }

    #[test]
    fn a_non_ok_playability_carries_its_status() {
        let mut v = player_ok("unused");
        v["playabilityStatus"] = serde_json::json!({ "status": "UNPLAYABLE", "reason": "made for kids" });
        match parse_player_response(&v, ClientIdentity::AndroidVr("1.62.20")) {
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
    fn every_android_vr_identity_is_below_the_gate_at_1_64() {
        for identity in IDENTITIES {
            if let ClientIdentity::AndroidVr(version) = identity {
                assert!(
                    ordinal(version) < ordinal("1.64.0"),
                    "{version} resolves OK and then plays nothing at all"
                );
            }
        }
    }

    /// One identity is a single point of failure against a bot check whose
    /// verdict moves between sessions.
    #[test]
    fn there_is_more_than_one_identity_to_fall_back_on() {
        assert!(IDENTITIES.len() > 1);
    }

    /// The 2026-08-20 outage. ANDROID_VR is refused on *both* sides of the
    /// 1.64 wall now — bot-checked below it, gated at and above it — so it
    /// cannot lead. Asking it first costs the user two refusals per track
    /// before anything that plays is even tried.
    #[test]
    fn visionos_is_asked_first() {
        assert_eq!(IDENTITIES.first(), Some(&ClientIdentity::VisionOs));
    }

    /// Every field was measured rather than guessed: an identity YouTube
    /// cannot place is answered exactly like a bot.
    #[test]
    fn the_visionos_identity_is_the_one_that_was_measured() {
        let ctx = ClientIdentity::VisionOs.context();
        assert_eq!(ctx["clientName"], "VISIONOS");
        assert_eq!(ctx["clientVersion"], "1.02");
        assert_eq!(ctx["deviceMake"], "Apple");
        assert_eq!(ctx["deviceModel"], "RealityDevice17,1");
        assert_eq!(ctx["osName"], "visionOS");
        assert_eq!(ClientIdentity::VisionOs.id(), "101");
        // Not an Android client: sending an Android field marks it as forged.
        assert!(ctx.get("androidSdkVersion").is_none());
    }

    /// The UA travels with the url to the media GET, so it must be the one
    /// VISIONOS presented — not an Oculus UA left over from the old client.
    #[test]
    fn a_visionos_target_carries_the_visionos_user_agent() {
        let t = parse_player_response(&player_ok("https://cdn/x"), ClientIdentity::VisionOs)
            .expect("itag 140 with a url must resolve");
        assert_eq!(
            t.headers,
            vec![("User-Agent".to_string(), VISIONOS_UA.to_string())]
        );
        assert!(!VISIONOS_UA.contains("oculus"));
    }

    /// Comparing `VISIONOS_UA` to itself proves nothing about its *text*, and
    /// a Rust `\` line-continuation swallows the next line's indentation — a
    /// UA in this crate has silently gained ten spaces that way before. So
    /// pin the exact string YouTube was measured accepting.
    #[test]
    fn the_visionos_user_agent_survived_its_line_continuation() {
        assert_eq!(
            VISIONOS_UA,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 \
             (KHTML, like Gecko) Version/26.0 Safari/605.1.15"
        );
        assert!(!VISIONOS_UA.contains("  "), "collapsed whitespace, not doubled");
    }

    #[test]
    fn the_user_agent_names_the_version_that_minted_the_url() {
        let t = parse_player_response(&player_ok("https://cdn/x"), ClientIdentity::AndroidVr("1.61.48"))
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
            parse_player_response(&serde_json::json!("not an object"), ClientIdentity::AndroidVr("1.62.20")),
            Err(NativeMiss::BadJson)
        ));
        assert!(matches!(
            parse_player_response(&serde_json::json!({}), ClientIdentity::AndroidVr("1.62.20")),
            Err(NativeMiss::BadJson)
        ));
    }

    /// A 140 with a url but no expire= still plays now — but must never enter
    /// the cache as immortal. parse_expiry returning None is the existing
    /// "immediately stale" posture; the target itself is still valid.
    #[test]
    fn a_url_without_expiry_resolves_with_none() {
        let t = parse_player_response(&player_ok("https://cdn/no-expiry"), ClientIdentity::AndroidVr("1.62.20")).unwrap();
        assert_eq!(t.expires_at, None);
    }
}
