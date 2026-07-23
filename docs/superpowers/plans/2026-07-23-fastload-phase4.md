# Fast-Load Phase 4 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cold first-click resolve ~5.3s → ~0.3–0.7s: one native InnerTube `player` POST (ANDROID_VR client — no PO token, no cipher, direct URLs) + one validation probe, with yt-dlp as the automatic fallback for every miss.

**Architecture:** New `innertube.rs` in hm-ytmusic (request + strict itag-140 parse into the existing `StreamTarget`), wired ahead of `resolve_with_fallback` in `YtMusicState::resolve` behind a probe-validated, fallback-on-anything gate with hit/miss telemetry. Nothing else moves.

**Tech Stack:** Rust (reqwest blocking via the Phase-2 shared probe client, serde_json), fixture-driven cargo tests + one live throttle-canary test.

**Spec:** `docs/superpowers/specs/2026-07-23-fastload-phase4-design.md`
**Analysis:** `.superpowers/sdd/fable-fastload-analysis.md` §R6

## Global Constraints

- Branch: `feat/fastload-phase4` (off main; Phases 1–3 merged).
- **Fallback-on-anything is a design requirement**: ANY native failure (HTTP, timeout, parse, playability, no itag 140, ciphered, probe refusal) falls through to the EXISTING `resolve_with_fallback` path unchanged — a native regression must be invisible to the listener.
- **Strict itag 140 only** — an Opus/WebM URL must be unrepresentable in the native path (the decoder pin; memory: bestaudio=Opus is silent in this player).
- The ANDROID_VR client constants live in ONE place with a doc comment naming them as the thing that rots; misses are typed so the log names why.
- Native path takes NO cookies (android_vr is anonymous by design; session content misses → yt-dlp with the cookie file, exactly today).
- Downloads + video renditions stay on yt-dlp. No frontend changes.
- Env escape hatch `HM_NATIVE_RESOLVE=0` disables the native attempt (read once per process).
- Repo rules: no `Co-Authored-By`; push only at the end. Run all commands from repo root.

---

### Task 1: `innertube.rs` — request shape + strict parse (fixture TDD)

**Files:**
- Create: `crates/hm-ytmusic/src/innertube.rs`
- Modify: `crates/hm-ytmusic/src/lib.rs` (`mod innertube;` next to the other mods)
- Modify: `crates/hm-ytmusic/src/ytdlp.rs` (`parse_expiry` → `pub(crate)`)

**Interfaces:**
- Produces (used by Task 2):

```rust
pub(crate) enum NativeMiss {
    Http(String),          // transport/status — includes timeouts
    BadJson,
    Unplayable(String),    // playabilityStatus.status != "OK" (carries the status)
    NoItag140,
    Ciphered,              // itag 140 exists but has signatureCipher / no url — the SABR canary
}
pub(crate) fn parse_player_response(json: &serde_json::Value) -> Result<ytdlp::StreamTarget, NativeMiss>;
pub(crate) fn resolve_native(client: &reqwest::blocking::Client, video_id: &str) -> Result<ytdlp::StreamTarget, NativeMiss>;
pub(crate) const ANDROID_VR_UA: &str; // also the StreamTarget's sole header
```

- [ ] **Step 1: Write the failing fixture tests**

In `innertube.rs`'s own `#[cfg(test)]` module (fixtures inline via `serde_json::json!` — small enough not to warrant files):

```rust
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
```

(Derive `Debug` on `NativeMiss` for the test matches.)

Run: `cargo test -p hm-ytmusic --no-run 2>&1 | tail -4` → COMPILE ERROR.

- [ ] **Step 2: Implement**

```rust
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

/// The client identity the `player` call presents — THE THING THAT ROTS.
/// When the native hit-rate collapses, refresh these from yt-dlp's
/// `_base_client` for android_vr (yt_dlp/extractor/youtube/_base.py) before
/// suspecting SABR.
const CLIENT_NAME: &str = "ANDROID_VR";
const CLIENT_VERSION: &str = "1.62.27";
/// Numeric id for the X-YouTube-Client-Name header (android_vr = 28).
const CLIENT_ID: &str = "28";
pub(crate) const ANDROID_VR_UA: &str =
    "com.google.android.apps.youtube.vr.oculus/1.62.27 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip";

#[derive(Debug)]
pub(crate) enum NativeMiss {
    Http(String),
    BadJson,
    Unplayable(String),
    NoItag140,
    Ciphered,
}

impl std::fmt::Display for NativeMiss { /* short reason strings for the tally log */ }

/// One `player` POST → a validated-shape (not yet probed) StreamTarget.
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
                "hl": "en", "gl": "US",
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
/// unpredictability doesn't — clock nanos are plenty.
fn cpn() -> String { /* base64url-ish encode of SystemTime nanos, 16 chars */ }

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
            .map(|b| (b / 1000) as u32),
        expires_at: ytdlp::parse_expiry(url),
    })
}
```

(Sketch — the implementer fills `Display` and `cpn()`; `parse_expiry` becomes `pub(crate)` in ytdlp.rs. If serde_json's `.json::<Value>()` needs the reqwest `json` feature, it is already in the workspace dep per Phase 2's review — verify, and if absent use `resp.text()` + `serde_json::from_str`.)

- [ ] **Step 3: Run** — `cargo test -p hm-ytmusic innertube 2>&1 | tail -4` → 6 passed (verify by exact names if the filter under-matches); crate suite + clippy clean.

- [ ] **Step 4: Commit** — `git add crates/hm-ytmusic/src/innertube.rs crates/hm-ytmusic/src/lib.rs crates/hm-ytmusic/src/ytdlp.rs && git commit -m "feat(ytmusic): native InnerTube player resolution — the fast path"`

---

### Task 2: Wire into `resolve` + telemetry + live canary

**Files:**
- Modify: `crates/hm-ytmusic/src/lib.rs` (`resolve` ~line 971; counters on `YtMusicState`; live test at the bottom)

**Interfaces:**
- Consumes: `innertube::{resolve_native, NativeMiss}`, Phase 2's `probe_client()` + `probe_ok()`.
- Produces: no public API change — `resolve`'s signature and fallback behavior are untouched.

- [ ] **Step 1: Counters + gate**

On `YtMusicState`: `native_hits: AtomicU64, native_misses: AtomicU64` (init 0). Module-level:

```rust
/// The dev escape hatch: HM_NATIVE_RESOLVE=0 turns the native fast path off
/// for a session (read once). The automatic fallback is the user-facing
/// safety mechanism; this exists for A/B-ing a misbehaving resolve in the
/// field without a rebuild.
fn native_resolve_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("HM_NATIVE_RESOLVE").map_or(true, |v| v != "0"))
}
```

- [ ] **Step 2: The resolve integration**

In `resolve`, between the cache check and the yt-dlp path:

```rust
        // The fast path: one InnerTube POST + one probe instead of a ~5s
        // yt-dlp process. Anything short of a probed, itag-140 url falls
        // through to yt-dlp exactly as before — a native regression must
        // never be more than a log line.
        if native_resolve_enabled() {
            match innertube::resolve_native(probe_client(), video_id) {
                Ok(target) if probe_ok(&target) => {
                    self.native_hits.fetch_add(1, Ordering::Relaxed);
                    self.remember_target(video_id, &target);
                    return Ok(target);
                }
                Ok(_) => self.native_miss("probe refused the url"),
                Err(miss) => self.native_miss(&miss.to_string()),
            }
        }
```

with:

```rust
    /// Count and log a native-resolve miss. One stderr line per miss keeps
    /// SABR/client-rot drift visible in Console output long before the
    /// fallback's slowness is noticed by ear.
    fn native_miss(&self, reason: &str) {
        use std::sync::atomic::Ordering;
        let misses = self.native_misses.fetch_add(1, Ordering::Relaxed) + 1;
        let hits = self.native_hits.load(Ordering::Relaxed);
        eprintln!("[hm-ytmusic] native resolve miss ({reason}) — {hits} hits / {misses} misses this session");
    }
```

- [ ] **Step 3: The live throttle canary**

Next to the other `--ignored` live tests:

```rust
    /// Run with `cargo test -p hm-ytmusic -- --ignored`. Needs network only —
    /// android_vr is anonymous by design (no keychain skip here).
    #[tokio::test]
    #[ignore = "requires network access"]
    async fn live_native_resolve_is_fast_and_unthrottled() {
        let t = tokio::task::spawn_blocking(|| {
            innertube::resolve_native(probe_client(), "dQw4w9WgXcQ")
        })
        .await
        .unwrap()
        .expect("android_vr should resolve a public track natively");
        assert_eq!(t.format_id, "140");
        assert!(t.expires_at.is_some(), "a googlevideo url carries expire=");
        // The canary: the url must serve real bytes at CDN speed. A silent
        // n-param/SABR regression pattern is a url that RESOLVES fine and
        // then serves at ~1x realtime — status checks alone would miss it.
        let start = std::time::Instant::now();
        let ok = tokio::task::spawn_blocking(move || {
            let mut req = probe_client().get(&t.url).header("Range", "bytes=0-65535");
            for (k, v) in &t.headers {
                req = req.header(k.as_str(), v.as_str());
            }
            req.send().map(|r| r.status().is_success() && r.bytes().map_or(0, |b| b.len()) >= 32_768).unwrap_or(false)
        })
        .await
        .unwrap();
        let took = start.elapsed();
        eprintln!("native 64KB fetch: {took:?}");
        assert!(ok, "the resolved url must serve ranged bytes");
        assert!(took < std::time::Duration::from_secs(2), "64KB took {took:?} — throttled?");
    }
```

- [ ] **Step 4: Verify**

`cargo test -p hm-ytmusic 2>&1 | tail -3` green; `cargo clippy -p hm-ytmusic --all-targets 2>&1 | tail -3` clean; `cargo check --workspace 2>&1 | tail -3` clean. Then the live tests: `cargo test -p hm-ytmusic -- --ignored 2>&1 | tail -8` — the new canary MUST pass (network permitting; a hard failure here means the client constants are already stale — STOP and report, do not ship a dead fast path).

- [ ] **Step 5: Commit** — `git add crates/hm-ytmusic/src/lib.rs && git commit -m "feat(ytmusic): resolve natively first, yt-dlp as the floor"`

---

### Task 3: Verification + push

- [ ] **Step 1: Gates** — workspace clippy; `cargo test -p hm-ytmusic -p hm-audio -p hm-core`; tsc; vitest; build. All green (hm-remote environmental).
- [ ] **Step 2: Push** — `git push -u origin feat/fastload-phase4`
- [ ] **Step 3: Memory** — update `hypemuzik_desktop_fastload.md`: Phase 4 done; note the constants-rot doc pointer, the miss-tally log line to watch, `HM_NATIVE_RESOLVE=0`, and the release-build check: cold-click a never-played track and stopwatch it (~1s vs ~6s), then check Console for the miss tally staying near zero.
