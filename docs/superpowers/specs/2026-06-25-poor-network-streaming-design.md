# Poor-network streaming (2G/3G robustness) — design

Date: 2026-06-25
Branch: `feat/crossfade-cloud-phone`
Status: Approved (pending spec review)

## Problem

Cloud and phone streaming today assumes a fast link. Two concrete failures on
slow/flaky networks (2G/3G):

1. **Full-download before playback (gapless queue).** The default cloud/phone
   path is `StreamQueueSource` (`crates/hm-audio/src/stream_queue.rs`), which
   downloads and decodes *each whole track into memory* before a single sample
   plays. On 2G/3G a 5–40 MB track is minutes of dead air per track.

2. **A dropped connection is treated as end-of-track (both paths).** In the
   progressive source `RadioStreamSource` (`crates/hm-audio/src/streaming.rs`),
   `decode_connection` returns `Stop::Eof` on *any* `next_packet()` error:

   ```rust
   let packet = match format.next_packet() {
       Ok(Some(p)) => p,
       _ => return Stop::Eof,   // any mid-stream network error → "track ended"
   };
   ```

   On a flaky link the connection drops mid-song, the worker sets
   `finished = true`, and the track cuts off early; the UI then advances or
   stops. There is no resume-from-byte-offset on an unexpected drop.

There is also no prebuffer policy (playback can start on a near-empty ring and
underrun immediately) and the ring is fixed at ~8 s.

## Goals

- Cloud/phone/radio playback **starts quickly and keeps playing** on 2G/3G.
- A dropped connection **resumes** instead of ending the track.
- Keep the existing gapless/crossfade experience **when the link can sustain it**.
- Give the user a **Data Saver / Low-bandwidth** override.

## Non-goals

- No transcoding / bitrate adaptation (we play the source file as-is).
- No gapless/crossfade *over* a constrained link (physically can't prefetch a
  whole next track in time; progressive single-track is the right behavior there).
- No change to local-file gapless playback (`queue.rs`), which is unaffected.

## Decisions (from brainstorming)

- **Adaptive**: gapless (`StreamQueueSource`) when the network is healthy;
  progressive single-track (`RadioStreamSource`) when it's slow or flaky.
- **Auto-detect + manual override**: auto-classify by measured throughput, with
  a user-facing **Data Saver** toggle that forces progressive + larger buffers.

## Architecture

```
                       ┌─ Data Saver ON ───────────────► progressive single-track
cloud/phone queue ─────┤
                       └─ Data Saver OFF (auto) ─► network classifier
                                                     ├─ "constrained" ► progressive single-track
                                                     └─ "fast"        ► gapless StreamQueueSource
```

- The **first stream of a session always starts progressively** — instant start,
  robust, and it measures the link.
- Once classified **fast**, later tracks/queues use the gapless source.
- A **rebuffer/underrun in gapless mode immediately downgrades** to progressive.

`RadioStreamSource` is the robust workhorse used by radio, single cloud/phone
tracks, mixed queues, **and** the progressive fallback. `StreamQueueSource` is
unchanged in shape (it already got the retry/connection-refresh hardening in the
prior fix) and is selected only for the "fast" case.

## Component design

### A. `RadioStreamSource` robustness — `crates/hm-audio/src/streaming.rs`

This is the core of Phase 1.

**A1. Resume-on-drop (the key correctness fix).**
- Wrap the per-connection HTTP reader in a **byte-counting reader** so the worker
  knows how many bytes it has pulled from the current connection
  (`bytes_this_conn`). Total consumed = `start_byte + bytes_this_conn`.
- When `decode_connection` stops, the worker decides *why* using the counter and
  the known body length (`content_bytes`):
  - **Genuine end**: `content_bytes` known and consumed ≥ `content_bytes` (or
    body length unknown and the stream ended cleanly after the retry budget) →
    `finished = true` (existing behavior).
  - **Drop**: still `running`, `content_bytes` known, consumed < `content_bytes`
    → re-open with `Range: bytes=<consumed>-`, **do not** set `finished`, and
    continue decoding. Backoff between attempts.
- A **consecutive-failure counter** bounds reconnection: if reconnects in a row
  make no forward progress (default cap 3 — e.g. a container format that can't
  re-probe mid-file, or a server that keeps closing), fall back once to
  `start_byte = 0`; if that also fails, give up and `finished = true`. The
  counter resets whenever a reconnect makes real forward progress. This prevents
  a hot reconnect loop.
- Mid-file resume re-probes from a byte offset, exactly like the existing seek
  path. It works cleanly for frame-aligned formats (MP3 / AAC-ADTS); for
  container formats that can't re-probe mid-file it degrades to the bounded
  reconnect-from-0 fallback. This matches the limitation seeking already has.

**A2. Prebuffer + rebuffer gate.**
- Add a `prebuffer_frames` target. `read()` produces **buffering silence**
  (counted as produced, so it's not EOF) until the ring first reaches the target
  or the stream finishes — a `started` latch flips once it's reached.
- **Rebuffer hysteresis**: if the ring fully drains while not finished, set a
  `rebuffering` flag and gate again until the ring refills to the target. This
  replaces choppy 1-sample-at-a-time underrun with clean "buffering… then resume."
- Targets: small on a fast link (~1–2 s), larger when constrained / Data Saver
  (~5–8 s). Passed in at construction from engine config.

**A3. Bigger ring.**
- Capacity ~8 s → ~30 s (configurable). Cost ≈ 30 s × 48 kHz × 2ch × 4 B ≈ 11 MB
  per active stream — negligible. Lets a slow link build a cushion.

**A4. Throughput metering (classifier input for Phase 2).**
- The decode worker measures bytes downloaded over wall-clock (`std::time::Instant`
  is fine on the worker thread) and publishes an EWMA `download_bps` (AtomicU64)
  plus a `rebuffer_count` (AtomicU32) on `StreamShared`. Exposed via the source so
  the engine/UI can read them. No policy in the source — policy lives in the
  store.

### B. Engine + IPC — `crates/hm-audio/src/engine.rs`, `src-tauri/src/`

- `EngineState.playback` gains `dataSaver: bool` (persisted with the rest of the
  engine state). Mirrored in the TS `PlaybackState` type.
- When constructing a `RadioStreamSource`, the engine passes prebuffer/ring config
  derived from `dataSaver` (Data Saver → larger prebuffer, no upfront prefetch).
- The forwarder thread (`src-tauri/src/lib.rs`) extends the existing
  `engine:progress` payload (already ~10 fps) with a `buffering: bool` and the
  estimate (`downloadBps`, `rebufferCount`). One channel, no new event type — the
  store already consumes progress, so the classifier reads it there.

### C. Frontend adaptive selection + UI — `src/stores/engine.ts`, settings, now-playing bar

- **Data Saver toggle** in playback settings (next to gapless/crossfade), wired to
  `engineSetPlayback` / a new setter; persisted.
- **Session network state** held in the store (module-scoped, like
  `gaplessQueueRunning`): `Unknown | Constrained | Fast`, updated from the
  published estimate and from rebuffer events.
- **`startPlayback` decision** for `source === "cloud" | "phone"` queues:
  - `dataSaver` → progressive single-track (never set `useCloudQueue`/`usePhoneQueue`).
  - else `Unknown` (first stream this session) → progressive single-track (instant
    + measures).
  - else `Fast` → gapless queue (current behavior).
  - else `Constrained` → progressive single-track.
  - Re-evaluated every `startPlayback`; a rebuffer in gapless mode flips state to
    `Constrained` so the next track is progressive.
- **Buffering indicator**: the now-playing bar shows "Buffering…" when the
  published `buffering` flag is set.

## Data flow

1. User plays a cloud/phone queue → `startPlayback` picks mode per §C.
2. Progressive mode: `cloudPlay`/`linkPlay` → engine builds `RadioStreamSource`
   with config from `dataSaver`. Worker streams, resumes on drops, meters
   throughput. `read()` gates on prebuffer, emits buffering silence on underrun.
3. Forwarder publishes progress + `buffering` + estimate each tick.
4. Store updates session network state; the *next* `startPlayback` may upgrade to
   gapless (Fast) or stay progressive (Constrained).
5. Gapless mode: existing `StreamQueueSource` path; a rebuffer downgrades the
   session state to Constrained.

## Error handling

- Connection drop mid-track → resume via Range (A1); never a silent track-end.
- Reconnect budget exhausted / un-resumable format → bounded fallback, then a
  real EOF so the queue advances instead of hanging.
- Resolve/HTTP failures already retried in `StreamQueueSource` (prior fix); the
  progressive source gets the same bounded-retry treatment on open.
- Data Saver and classifier only choose a *mode*; neither can wedge playback —
  the worst case is "progressive single-track," which always works.

## Testing

- **A1 resume**: unit-test the "drop vs end" decision — given (content_bytes,
  consumed, running) it returns Resume(offset) vs Finish vs FallbackReconnect.
  Pure function, no network.
- **A2 prebuffer/rebuffer**: drive the `read()` gate with a fake ring — silence
  until target reached (latch), rebuffer on drain, resume on refill. Counted as
  produced (not EOF) throughout.
- **A1 byte counter**: counting reader reports exact bytes for a scripted reader.
- **C decision table**: unit-test `startPlayback` mode selection across
  {dataSaver, Unknown, Fast, Constrained} → expected source path.
- Existing `streaming.rs` tests (id3v1, byte_offset, to_stereo) stay green.
- Manual: throttle to 2G/3G (macOS Network Link Conditioner) and confirm a song
  plays through a forced Wi-Fi drop without cutting off.

## Phasing

**Phase 1 — the actual poor-network fix (ship first):**
- A1 resume-on-drop, A2 prebuffer/rebuffer, A3 bigger ring, bounded open retries.
- `dataSaver` flag + settings toggle.
- `startPlayback`: progressive when `dataSaver` on **or** not-yet-classified-Fast.
- This alone makes 2G/3G work.

**Phase 2 — adaptive polish:**
- A4 throughput metering + `buffering`/estimate events.
- Session network classifier + auto-upgrade to gapless on Fast links + auto
  downgrade on rebuffer.
- Buffering indicator in the now-playing bar.

## Risks / open items

- Mid-file resume quality depends on container format (MP3/AAC good; MP4/FLAC
  may fall back to reconnect-from-0). Acceptable; matches existing seek limits.
- Throughput classification is heuristic; the Data Saver override is the
  deterministic escape hatch.
- `std::time::Instant` is used only on worker threads (not the RT audio callback,
  not workflow JS), so it's safe.
