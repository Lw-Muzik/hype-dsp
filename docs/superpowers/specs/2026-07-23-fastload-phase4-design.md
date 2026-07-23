# Fast YT Music Track Loading — Phase 4 Design

Date: 2026-07-23
Status: Approved (Phase 4 — final phase of the fast-load program; analysis §R6
in `.superpowers/sdd/fable-fastload-analysis.md`; Phases 1–3 shipped on main)

## Goal

The cold first click on a never-played track resolves in **~0.3–0.7s** (one
HTTPS POST + one probe) instead of ~5.3s (a yt-dlp Python process). Combined
with Phases 1–3 this is YT-Music-web-parity everywhere. yt-dlp remains the
fallback for every native miss and the only path for downloads and video
renditions.

## Why android_vr (2026 landscape, verified by the analysis)

- The `ANDROID_VR` InnerTube client requires **no PO token** (yt-dlp's PO
  Token Guide lists it exempt for both GVS and player) and returns **direct
  `url`s** — no `signatureCipher`, no n-param JS. It is yt-dlp's own
  first-choice default, and this repo's live-resolved URLs already carry
  `c=ANDROID_VR`: the native call fetches the same URL the subprocess
  fetches, minus the interpreter.
- The web client family needs PO tokens + a JS runtime for sig/n deciphering
  (a maintenance treadmill this app must not board); `tv` returns DRM-wrapped
  formats. android_vr takes no cookies, so private/library-only content
  cannot resolve on it — those miss and fall back to yt-dlp with the cookie
  file, which is exactly today's path.
- **SABR is the structural risk**: YouTube has removed plaintext
  `adaptiveFormats` from the web client and is pushing SABR/UMP elsewhere.
  When android_vr eventually loses direct URLs, the fast path must degrade to
  yt-dlp **silently** — fallback-on-anything is a design requirement, not
  politeness. A per-miss log line + running hit/miss tally makes the drift
  visible in Console output long before users notice.

## Architecture

### 1. `crates/hm-ytmusic/src/innertube.rs` (new)

- One blocking POST to `https://youtubei.googleapis.com/youtubei/v1/player`
  with the ANDROID_VR client context (clientName/clientVersion/matching
  User-Agent — all constants in ONE place with a doc comment naming them as
  the thing that rots), `videoId`, and a fresh `cpn` (16 URL-safe chars from
  SystemTime nanos — no new deps). 3s per-request timeout on the shared probe
  client (Phase 2's `probe_client()`).
- Parse: require `playabilityStatus.status == "OK"` (else miss with the
  status as the reason); walk `streamingData.adaptiveFormats`; **filter
  strictly `itag == 140`** (audio/mp4 — preserves the decoder pin; an Opus
  URL must be unrepresentable); require a plaintext `url` field (a
  `signatureCipher`-only format → miss "ciphered" — the SABR canary).
- Build a `ytdlp::StreamTarget`: the client's User-Agent as the sole header,
  `ext "m4a"`, `format_id "140"`, `abr_kbps` from the format's `bitrate` if
  present, `expires_at` via the existing `parse_expiry` (made `pub(crate)`).
- Misses are a typed enum (`NativeMiss`: Http (transport/status/timeouts) /
  BadJson / Unplayable{status} / NoItag140 / Ciphered) so the log line names
  WHY.

### 2. Wiring into `YtMusicState::resolve` (audio path only)

Cache check (unchanged) → **native attempt** → on success, **validate with
one `Range: bytes=0-1` probe** (reuses Phase 2's `probe_ok`; also warms TLS
toward googlevideo) → `remember_target` → done. ANY failure at ANY stage
(HTTP, parse, no itag 140, ciphered, probe refusal) → the existing
`resolve_with_fallback` yt-dlp path, completely unchanged, including the
`session_first` ordering machinery.

- Hit/miss counters (two `AtomicU64`s on `YtMusicState`) + one `eprintln!`
  per native miss with the reason and the running tally (the crate has no
  logging framework; stderr reaches Console.app — same visibility class as
  the crate's live tests). Hits stay silent.
- Escape hatch: env var `HM_NATIVE_RESOLVE=0` disables the native attempt
  entirely (read once). No UI setting — the automatic fallback IS the safety
  mechanism; the analysis's "ship behind a setting" is deliberately downgraded
  to this dev-facing hatch (YAGNI: a user-facing toggle for an invisible
  optimization with automatic fallback would be noise).
- `prefetch`/`prefetch_batch` inherit the fast path for free (they call
  `resolve`).

### 3. Explicitly unchanged

Downloads and the video rendition stay on yt-dlp. The probe/quarantine disk
cache (Phase 2), the gapless streaming worker (Phase 3), cookie handling, and
`resolve_with_fallback` are untouched. No frontend changes.

## Testing

- Fixture-driven parse tests (canned player JSON): happy path builds the
  exact StreamTarget (url/UA-header/ext/format_id/expiry); non-OK
  playabilityStatus → Unplayable miss; itag-140-absent → NoItag140;
  `signatureCipher`-only → Ciphered; garbage JSON → BadJson; each miss reason
  distinct.
- Integration: `resolve` prefers native (fixture-level test of the ordering
  is impractical — network; covered by the live test + the miss-path unit
  seam `native_then_fallback` decision if extractable).
- One `#[ignore]` live test (visible keychain-independent — android_vr needs
  no auth): native-resolve a real track; assert itag 140/m4a, `expire=`
  present, `probe_ok` passes, AND a ranged fetch of the first 64KB completes
  in < 2s — the throttle canary that catches a silent n-param/SABR
  regression that plain status checks would miss.
- Whole-workspace gates as usual.
