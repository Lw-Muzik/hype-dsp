# Endless Radio Auto-Queue (YT Music-style) — Design

Date: 2026-07-22
Status: Approved

## Goal

When the user plays a song from YT Music search (or an Explore shelf), the queue
becomes that song plus an endless stream of similar tracks picked by YouTube
Music's own radio algorithm, replenished forever — until the user plays
something else, which restarts the process from the new seed. YT playlist /
album / artist queues extend with radio when they reach their end instead of
stopping. Scope is YT Music tracks only; local / phone / cloud playback is
untouched.

## Why this works: the real algorithm, not an imitation

YT Music's radio is served by the InnerTube `next` endpoint with playlist id
`RDAMVM<videoId>`. Our ytmapi-rs 0.3.2 dependency already models this as
`GetWatchPlaylistQuery`, and upstream live-tests endless paging through
`nextRadioContinuationData` continuation tokens. Calling it with our existing
BrowserToken personalizes results to the signed-in account. Full endpoint
research (request shapes, JSON paths, gotchas) is recorded in the project
memory note `hypemuzik_desktop_ytmusic_radio.md`.

## Architecture

Approach chosen: real radio endpoint + continuation tokens, with frontend-owned
replenishment (the queue lives in `src/stores/engine.ts`, as it does for every
other source).

### 1. Rust — `crates/hm-ytmusic/src/radio.rs` (new)

- `RadioBatch { tracks: Vec<YtTrack>, continuation: Option<String> }`
- `radio(video_id) -> Result<RadioBatch>` — `json_query(GetWatchPlaylistQuery::
  new_from_video_id(id))` (auto-builds `RDAMVM<videoId>`; note: `next` takes the
  raw playlist id, NO `VL` prefix — the opposite of `browse`).
- `radio_continue(token) -> Result<RadioBatch>` — re-POST the same body with
  `?ctoken=<token>&continuation=<token>`, mirroring the existing playlist
  continuation-request pattern in `lib.rs` (~line 899). Parses
  `continuationContents.playlistPanelContinuation`; falls back to the full
  first-page panel path if `continuationContents` is absent (muse's defensive
  fallback).
- Hand-parse the `playlistPanelRenderer` with our `nav.rs` helpers, tolerant
  per-row (a bad row is skipped, never kills the batch). ytmapi-rs's typed
  parser hard-fails a page on one odd item and does not skip `unplayableText`
  rows, so it is unusable for production here.
- Rows are either `playlistPanelVideoRenderer` or
  `playlistPanelVideoWrapperRenderer` (unwrap `.primaryRenderer...`); a real
  capture had 42/50 wrapped. Read `musicVideoType` for `hasVideo`.
- Skip: the seed row (`contents[0]`, `selected: true`) and any row with
  `unplayableText`. Map into the existing `YtTrack`
  (`playlistTitle: "Radio"`, `playlistId` = the radio playlist id).
- Hold the continuation chain; never re-issue page 1 expecting the same queue
  (radio regenerates per call).

### 2. Tauri commands — `src-tauri/src/commands/ytmusic.rs`

- `ytmusic_radio(video_id: String) -> RadioBatch`
- `ytmusic_radio_continue(token: String) -> RadioBatch`

Async, same auth/client plumbing as the existing ytmusic commands.

### 3. Engine store — radio session (`src/stores/engine.ts`)

- Session state: `radio: { seedId, continuation, fetching, exhausted } | null`.
- `playYtRadio(seed: YtTrack)`: queue = `[seed]`, play immediately, fetch the
  first batch in the background and append (~25–50 tracks appear in Up Next
  while the seed plays).
- Low-water replenishment: whenever the queue advances, if a radio session is
  live, autoplay is on, and ≤5 unplayed tracks remain ahead → fetch the
  continuation and append, deduped by videoId against the whole queue
  (continuation pages can overlap).
- New internal `appendQueueItems`: extends both `queue` and `order`; appended
  linearly even under shuffle (matches YT Music).
- Session teardown: any play action that replaces the queue clears the radio
  session. Toggling autoplay off keeps already-queued tracks but stops
  extending.
- Autoplay-extend: an all-ytmusic queue (playlist/album/artist) reaching its
  end with repeat off starts a radio session seeded from its last track,
  appending rather than replacing.
- Endless guarantee: if a batch returns no continuation token (rare — radio
  panels normally always return one), re-seed a fresh radio from the last
  auto-added track, deduped. Radio only truly stops if YT returns nothing.

### 4. Trigger points — `src/stores/explore.ts`

- Search song/video click → `playYtRadio(seed)` (replaces the current
  "queue the other search results" behaviour).
- Explore shelf single-song click → same.
- Playlist/album/artist track click → unchanged (list queue); radio picks up
  at the end via autoplay-extend.

### 5. Autoplay toggle

- `autoplay: bool` on `PlaybackState` — Rust `crates/hm-core/src/types.rs`
  with a serde default of `true` (existing saved states keep working), mirrored
  in `src/lib/types.ts`. Default ON; persisted via the existing EngineState
  autosave.
- UI: an "Autoplay" switch in the queue/Up-Next panel; a thin divider in the
  queue where auto-added tracks begin.

### 6. Error handling

Radio fetches never interrupt playback: on failure, one silent retry, then give
up quietly (the queue simply ends, as today). Console-logged, no toasts.

### 7. Testing

- Rust: fixture tests for the radio page, a continuation page, wrapper
  renderers, malformed-row tolerance, `unplayableText` skip, seed skip; one
  `--ignored` live test with a visible skip when keychain auth is absent (the
  silent-skip false-green trap from the search work).
- Vitest: seed+append flow, low-water trigger at ≤5 remaining, dedup, session
  teardown on new queue, autoplay-off gating, end-of-playlist extension,
  shuffle append ordering.
