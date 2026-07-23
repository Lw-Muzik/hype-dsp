# Fast YT Music Track Loading — Phase 3 Design

Date: 2026-07-23
Status: Approved (Phase 3 of the fast-load program; analysis §R5 in
`.superpowers/sdd/fable-fastload-analysis.md`; Phases 1–2 shipped on main)

## Goal

The gapless queue (the DEFAULT playback path for YT/cloud/phone queues) starts
the current track after ~1 second of decoded audio exists instead of after the
whole track has downloaded AND decoded. Warm gapless starts: ~1.5–3s →
**~0.3–0.8s**. This is the one structural refactor of the program.

## Why it's slow today

`StreamQueueSource` is all-or-nothing per track: `fetch_once` downloads the
entire body to a spool file, `decode_file` decodes the entire file to PCM,
`resample_stereo` resamples the entire buffer, and only then does the track
slot become `Some(samples)` and playable. Post-Phase-1 the download runs at
CDN speed, but a full song is still ~1–3s of download plus ~0.3–1s of decode,
serialized, before the first audible frame.

## Architecture: incremental publish over the existing channel

The read side (`read()` runs on the audio pull path) stays lock-free: the
worker keeps sending over the existing mpsc channel and `drain()` keeps
applying with `try_recv`. No shared-state locks are introduced — the channel
message just gets richer.

### 1. `StreamResampler` — chunk-safe resampling (`decode.rs`)

`resample_stereo` interpolates over one whole buffer; applied naively per
chunk it would clamp interpolation at every chunk edge (an audible
discontinuity per chunk). New `StreamResampler { src_rate, dst_rate, ... }`
with `push(&mut self, chunk: &[f32]) -> Vec<f32>` and `finish(&mut self) ->
Vec<f32>`, carrying the interpolation anchor and output-frame index across
chunks using the SAME arithmetic as `resample_stereo` (global output index →
global source position), so that any chunking of an input produces
**bit-identical** output to the one-shot function. Property-tested against
`resample_stereo` as the reference across several rates and split points.

### 2. Chunked decode (`decode.rs`)

`decode_format`'s packet loop already produces audio incrementally; it just
accumulates privately. Refactor into `decode_format_chunked(format,
min_chunk_frames, sink: &mut dyn FnMut(DecodeChunk) -> bool)` where
`DecodeChunk` is:
- `Meta(TrackMeta, sample_rate)` — sent once, right after probe (tags + cover
  are front-loaded; this also lets the UI announce the track seconds earlier);
- `Pcm(Vec<f32>)` — source-rate interleaved stereo, flushed whenever ≥
  `min_chunk_frames` have accumulated;
- sink returning `false` aborts the decode (queue torn down).

`decode_file`/`decode_bytes` become thin wrappers that collect chunks — one
decode path, existing callers and tests untouched. Equivalence pinned by a
test decoding a generated WAV both ways (bit-identical samples + meta).

### 3. Slot state machine (`stream_queue.rs`, read side)

`tracks: Vec<Option<Vec<f32>>>` becomes `Vec<Slot>`:

```
enum Slot {
    Empty,                          // was None: not decoded (buffer silence)
    Growing { samples: Vec<f32> },  // NEW: PCM still arriving (device rate)
    Done { samples: Vec<f32> },     // was Some: complete; empty = failed/skip
}
```

Channel message `DecodedTrack` becomes:

```
enum DecodeEvent {
    Meta { idx, meta },        // announce early
    Chunk { idx, samples },    // device-rate PCM, appended in order
    Done { idx },              // Growing -> Done (empty Growing -> skip)
    Failed { idx },            // -> Done(empty) = skip
    Reset { idx },             // discard buffered PCM (retry); cursor KEPT
}
```

`Meta`'s early announce is gated by `xf_len`, not applied unconditionally: it
re-signals the now-playing UI only when `idx == self.index && xf_len == 0`.
Mid-crossfade, the incoming track was already announced at the fade's own
start (`signal_index(self.index + 1)`); re-signalling the outgoing current
index's `Meta` on top of that would wrong-foot the UI back to the track
that's fading out until the next real change. The internal bookkeeping
(`self.metas[idx]`) still updates either way — only the UI-facing signal is
suppressed mid-fade.

`read()` rules (the correctness core — each is a test):
- **Start gate**: the current track is playable when `Done`, or `Growing`
  with ≥ `START_FRAMES` (1s at device rate). Below that: buffer silence
  (`produced += 1`), exactly like today's undecoded case.
- **Underrun**: `Growing` and `cursor >= len` → buffer silence, do NOT
  advance (the track isn't over, the network is behind). Boundary advance
  (index += 1) happens ONLY on `Done && cursor >= len`.
- **Crossfade gates**: a crossfade may start only when the current track is
  `Done` (otherwise its true tail — and therefore the fade point — is
  unknown) AND the next is `Done` or `Growing` with a buffered head of at
  least `xf.max(start_frames)` — the fade width, but never less than the
  ~1s start gate itself (shipped as `growing_head_at_least(idx,
  xf.max(start_frames(device_rate)))`). The fade width alone isn't enough:
  trusting a `Growing` head only as wide as the fade would let a track that
  clears the fade but not the start gate begin fading in, then immediately
  fail `ready()` and cut to silence one frame after the boundary — a worse
  seam than the deferred fade this mechanism exists to avoid. Flooring the
  trust threshold at `start_frames` keeps audibility monotone across the
  boundary (once it's audible, it never becomes silence again on its own).
  Otherwise defer; the existing `xf_len` latch already handles late
  lookaheads degrading to shorter fades.
- **Reset**: empties the slot's buffer back to `Growing(empty)` but KEEPS the
  cursor — the listener's position survives a mid-track retry; playback
  stalls on silence until the re-decode passes the cursor again (today's
  rebuffer experience).
- `seek` keeps its clamp-to-len contract; `position`/`total_frames` report
  the growing length. This has a frontend consequence the read side alone
  can't fix: `engine:progress`'s own reported duration grows right along
  with it (~1s climbing toward the real length as a streamed track
  decodes), and the store's naive `p.durationSecs ?? s.durationSecs` used
  to adopt that number the instant it arrived — visibly counting the seek
  bar's total up from 0:01 to the real length on every streamed track (and
  re-publishing OS media metadata on each change). Fixed store-side
  (`reconcileDuration` in `src/stores/engine.ts`): trust the queue item's
  own known duration until the engine's growing total reaches or exceeds
  it, then adopt the engine's number and keep it.
- Memory bound unchanged: played slots free to `Empty`, ~2 tracks resident.

### 4. Worker: overlap download and decode (`stream_queue.rs`)

The worker decodes **from the HTTP response as it arrives**, teeing every
byte into the spool file as it reads:
- `TeeReader<R>` wraps the response body: `read()` passes bytes through and
  appends them to the spool file. Symphonia consumes it as a non-seekable
  `MediaSourceStream` (`ReadOnlySource`).
- Chunks flow: source-rate PCM from `decode_format_chunked` → per-track
  `StreamResampler` → `DecodeEvent::Chunk` at device rate.
- **Container fallback**: DASH m4a (itag 140) is forward-readable in
  practice, but if the streaming probe/decode fails where a whole-file decode
  might not (symphonia's isomp4 reader can demand seeking for some layouts),
  fall back: finish downloading the body into the spool (the tee already has
  every byte read so far), then decode the complete spool file exactly as
  today, publishing `Chunk`s from it. The fallback decision is a pure
  function of the error + download state, unit-tested.
- **Retry semantics**: a failure before any chunk was published behaves
  exactly as today (Retry/Skip through `load_with_retry`). After partial
  publish: send `Reset { idx }`, then retry the whole track through the same
  ladder. `fresh=true` re-resolution on attempt >1 is unchanged.
- Only the CURRENT track benefits from eager starting; lookaheads decode the
  same way (chunks) but nothing reads them early — no behavior change there.

### 5. What does NOT change

The engine/AudioSource contract, `queue.rs`, the resolver closures in
src-tauri, the progressive path (`streaming.rs`), crossfade gain math, the
`load_with_retry` ladder shape, spool lifecycle (delete-on-drop), and the
Phase-1/2 behaviors (always-Range, shared client, per-request `FETCH_TIMEOUT`
— the tee reads under the same 90s per-request cap; a track longer than the
cap downloads within it at CDN speed or was already failing today).

## Risks & mitigations

- **Symphonia over a non-seekable growing stream**: mitigated by the
  container fallback (worst case = exactly today's behavior, per track).
- **Chunk-boundary resampling artifacts**: eliminated by construction
  (bit-identical property test).
- **Audio-thread allocations**: `drain()` already moved whole-track `Vec`s on
  this path; appending ~1s chunks (`extend_from_slice`) is strictly smaller
  work per call. Accepted as the file's existing discipline.
- **A Growing track that never finishes** (stalled link): the per-request 90s
  cap eventually fails the fetch → Reset + retry ladder → Skip after
  MAX_ATTEMPTS — same terminal behavior as today, now with the head already
  heard.

## Testing

- `StreamResampler`: bit-equality vs `resample_stereo` across rates
  (44.1→48k, 48→44.1k, identity) and adversarial split points (1-frame
  chunks, uneven splits).
- Chunked decode: generated-WAV equivalence (chunked collect ≡ `decode_file`)
  including meta; abort-on-false sink.
- Read-side state machine: extend the eager test harness with `Growing`
  slots — start gate, underrun-no-advance, boundary-only-on-Done, crossfade
  deferral + Growing-head crossfade, Reset-keeps-cursor, skip on
  `Done(empty)`. ALL existing gapless/crossfade tests must pass unchanged
  (they construct `Done` slots).
- Worker: `TeeReader` pass-through + spool-completeness; fallback decision
  table.
- **Shipped in place of the promised `--ignored` live latency test**: two
  real end-to-end tests drive `stream_decode_attempt` over a local TCP
  harness with a real, probeable WAV body — a clean complete stream
  (asserts the exact `Meta`/`Chunk*`/`Done` event sequence and a `Published`
  return) and a body shorter than its declared `Content-Length` (asserts no
  `Done` is ever published, exactly one `Reset`, and a `Retry` return). Both
  exercise the real function and the real event protocol end to end, just
  without a network YT fetch or a latency assertion, which would be flaky
  against real network variance. What an automated test can't practically
  cover (audible quality, real-network start latency) is left to the
  release-build manual checklist the plan already calls for (Task 5, Step
  3): gapless YT queue start latency by ear, crossfade correctness at batch
  boundaries, throttled-network stall/resume with no skip or crash, and
  seeking during a still-growing track. That checklist is the verification
  path for this work, not an automated substitute — running it is a
  release-build step, not part of this test suite.
  - **Not implemented, left as future work**: (1) a hand-built moov-at-tail
    (non-faststart) mp4/m4a fixture to exercise the container-fallback path
    (`DecodeSpool`) through this same harness — WAV/PCM has no seek
    dependency for a well-formed file, so it can't provoke a streaming-probe
    failure whose complete body still decodes whole; the fallback's
    *classification* logic is fully pinned at the pure-fn level instead
    (`after_stream_failure`/`drain_then_classify`), and its wiring is
    identical code to the already-tested truncation-recovery path. (2) the
    originally-scoped `--ignored` live test against a real YT track,
    asserting first-chunk latency < full-track time.
- Whole-workspace gates as usual.
