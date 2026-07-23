# Fast YT Music Track Loading — Phase 1 Design

Date: 2026-07-23
Status: Approved (Phase 1 of the fast-load program; analysis in
`.superpowers/sdd/fable-fastload-analysis.md`)

## Goal

Cut the perceived start latency of YT Music playback for every *warm* or
*queued* interaction — queue advance, skip, replay, radio auto-append — from
multi-second to **~0.5–1s**, without touching the cold-first-click resolve
(that's Phase 4's native-InnerTube work). Three independent, individually
shippable changes; hours of work, near-zero risk.

## R1 — Send `Range: bytes=0-` on the first progressive open

googlevideo paces a plain GET to ~1× realtime (a measured 190× throughput
penalty); any request with a `Range` header is served at full speed. The
progressive path's `open()` (`crates/hm-audio/src/streaming.rs:873-897`) adds
`Range` only when `start_byte > 0`, so the FIRST open of every progressive
stream — the one the listener is waiting on — is the slow one. The gapless
sibling learned this long ago (`stream_queue.rs` `fetch_once` sends `bytes=0-`
and has a wire-level regression test).

Change: `open()` always sends `Range: bytes={start_byte}-`.
Acceptance rules: at `start_byte == 0` accept **200 or 206** (some
radio/Icecast servers ignore Range; a 200 at byte 0 is exactly today's
behavior); for `start_byte > 0` keep requiring 206 exactly as today (a 200
would replay the body from byte 0). Clone the sibling's wire test for this
path: a hyper test server asserting the header arrives on the first request,
plus 200-at-zero / 206-at-zero / 200-at-offset-rejected cases.

Also speeds seek re-opens and cloud/phone progressive streams for free.

## R2 — Batch pre-resolution of the upcoming queue

The queue's stream-URL lookahead is exactly 1, so one slow resolve = an
audible gap, and a user skipping twice outruns it. Resolved URLs live ~6h in
the in-memory cache (`prefetch` is idempotent against it).

Change: one new Tauri command `ytmusic_prefetch_batch(video_ids: Vec<String>)`
(`#[tauri::command(async)]`, mirroring `ytmusic_prefetch`) that walks the ids
**sequentially** — each prefetch is a full yt-dlp process; two concurrent
spawns visibly contend — skipping fresh cache entries via the existing
`prefetch` idempotence. Fire-and-forget contract, same as its siblings.

Call sites (frontend, which owns the queue):
- When a YT engine-gapless queue starts: warm the 3 tracks after `start`.
- When a `RadioBatch` appends to the queue: warm the first 3 appended tracks.
Both deferred (see R3) so they never contend with the click's own resolve.
Depth 3 is the ceiling — deeper buys nothing until Phase 4 makes resolves
cheap.

## R3 — Get the competing spawns off the click

A click on a video-capable track currently fires up to 3 concurrent yt-dlp
spawns (audio resolve + video prefetch + next-track prefetch), contending for
CPU/network at the exact moment the listener is waiting.

Change: in `startPlayback`'s ytmusic branch (`src/stores/engine.ts`), defer
`ytmusicVideoPrefetch` and the single-track next-`ytmusicPrefetch` (and R2's
batch calls) by ~3s via `window.setTimeout`, cancelling the pending timer when
a newer track starts (skip-spam must not stack timers). Pure scheduling; all
calls are already fire-and-forget.

## Explicitly out of scope (later phases)

Disk-persisted URL cache + revalidation probe (Phase 2), incremental gapless
start (Phase 3), native InnerTube resolve (Phase 4).

## Testing

- R1: Rust wire tests as above (the sibling's test is the template).
- R2: command is thin plumbing over the idempotent `prefetch`; Rust unit test
  not required beyond compile + clippy. Frontend scheduling covered by R3's.
- R3: extract the defer/cancel scheduling into a small pure helper
  (`src/stores/warmup.ts`: schedule(fn, delay) returning a cancel handle,
  latest-wins per key) and unit-test it (timer fake via vi.useFakeTimers);
  engine wiring stays thin.
- Whole-workspace gates as usual: clippy, cargo test (touched crates), tsc,
  vitest, build.
