# Fast-Load Phase 3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The gapless queue starts the current track on ~1s of decoded PCM instead of after full download + decode: warm starts ~1.5–3s → ~0.3–0.8s.

**Architecture:** Four increments: (1) `StreamResampler` — chunk-safe resampling bit-identical to the one-shot reference; (2) `decode_format_chunked` — the existing packet loop refactored to emit Meta + PCM chunks, with `decode_file`/`decode_bytes` as collecting wrappers; (3) the read-side `Slot` state machine (`Empty`/`Growing`/`Done`) + `DecodeEvent` channel protocol, with an interim whole-track worker so the crate stays green; (4) the worker overlap — decode from the HTTP body through a spool-teeing reader, with a container fallback to today's download-then-decode and truncation-safe retry.

**Tech Stack:** Rust (symphonia, reqwest blocking, mpsc), cargo test with the eager harness + real-TCP wire tests.

**Spec:** `docs/superpowers/specs/2026-07-23-fastload-phase3-design.md`
**Analysis:** `.superpowers/sdd/fable-fastload-analysis.md` §R5

## Global Constraints

- Branch: `feat/fastload-phase3` (off main; Phases 1–2 merged).
- The read side stays lock-free: worker→source communication is ONLY the existing mpsc channel via `drain()`/`try_recv`. No `Mutex`/`RwLock` on the `read()` path.
- Correctness rules from the spec §3, verbatim: start gate = `Done` or `Growing ≥ START_FRAMES` (1s at device rate); underrun on `Growing` buffers silence and never advances; boundary advance ONLY on `Done && cursor >= len`; crossfade only from a `Done` current into a `Done`-or-`Growing≥fade-width` next; `Reset` empties the buffer but KEEPS the cursor; `seek` keeps its clamp-to-len contract; played slots free to `Empty`.
- **Truncation guard survives streaming**: an `Ok`-short byte count (fewer bytes seen than the server declared) doesn't classify on its own — it first *drains* the rest of the response (reading, not decoding, whatever's left) before deciding: draining completes the count → a genuine end of track, even though the decoder itself hit a clean EOF early (e.g. a RIFF/WAV trailing chunk after the audio data it never needed to read) — trusted `Done`. Draining still comes up short → a real transport truncation → `Reset` (if chunks were published) then `Retry`, never a short `Done`. This is the clipped-track bug the old `fetch_once` explicitly guarded; the streaming path must keep the guarantee (shipped as `finish_or_drain_then_retry`).
- ALL existing stream_queue/decode tests pass unchanged (they construct complete tracks → `Done` slots).
- `StreamResampler` output must be bit-identical to `resample_stereo` for any chunking of the same input.
- Retry/Skip classification (5xx/429/408→Retry, other non-success→Skip, transport error→Retry) and `fresh=true` on attempt >1 are preserved exactly.
- Repo rules: no `Co-Authored-By`; push only at the end. Run all commands from repo root.

---

### Task 1: `StreamResampler` (chunk-safe resampling)

**Files:**
- Modify: `crates/hm-audio/src/decode.rs` (below `resample_stereo` ~line 211; tests in the existing test module)

**Interfaces:**
- Produces (used by Task 4): `pub struct StreamResampler` with `pub fn new(src_rate: u32, dst_rate: u32) -> Self`, `pub fn push(&mut self, chunk: &[f32]) -> Vec<f32>`, `pub fn finish(&mut self) -> Vec<f32>`.

- [ ] **Step 1: Write the failing tests**

```rust
    /// Chunked resampling must be indistinguishable from one-shot: same
    /// arithmetic, same output, regardless of where the input was split.
    fn assert_chunked_matches(input: &[f32], src: u32, dst: u32, splits: &[usize]) {
        let reference = resample_stereo(input, src, dst);
        let mut r = StreamResampler::new(src, dst);
        let mut out = Vec::new();
        let mut at = 0;
        for &n in splits {
            let end = (at + n * 2).min(input.len());
            out.extend(r.push(&input[at..end]));
            at = end;
        }
        out.extend(r.push(&input[at..]));
        out.extend(r.finish());
        assert_eq!(out.len(), reference.len(), "same frame count");
        assert!(
            out.iter().zip(&reference).all(|(a, b)| a == b),
            "chunked output must be bit-identical to the one-shot reference"
        );
    }

    fn ramp(frames: usize) -> Vec<f32> {
        (0..frames * 2).map(|i| i as f32 / 100.0).collect()
    }

    #[test]
    fn chunked_resample_matches_one_shot_upsampling() {
        assert_chunked_matches(&ramp(1000), 44_100, 48_000, &[1, 7, 100, 250]);
    }

    #[test]
    fn chunked_resample_matches_one_shot_downsampling() {
        assert_chunked_matches(&ramp(1000), 48_000, 44_100, &[3, 500, 1]);
    }

    #[test]
    fn chunked_resample_is_identity_at_same_rate() {
        assert_chunked_matches(&ramp(64), 48_000, 48_000, &[10, 10]);
    }

    #[test]
    fn one_frame_chunks_still_match() {
        assert_chunked_matches(&ramp(48), 44_100, 48_000, &[1; 40]);
    }
```

Run: `cargo test -p hm-audio chunked_resample 2>&1 | tail -4` → COMPILE ERROR.

- [ ] **Step 2: Implement**

The reference (`resample_stereo`) computes, for each output frame `i`: `src_pos = i / ratio`, then linearly interpolates between `floor(src_pos)` and `floor(src_pos)+1` (both clamped to the last frame). To be bit-identical, `StreamResampler` must use the SAME global arithmetic — global output index `i`, global source frame count — never per-chunk positions:

```rust
/// Chunk-at-a-time [`resample_stereo`]: same arithmetic, same output,
/// bit-for-bit, no matter where the input is split.
///
/// Works on GLOBAL indices — the next output frame `i` and the total source
/// frames seen — so a chunk boundary never becomes an interpolation boundary.
/// Emits output frame `i` only once source frame `floor(i/ratio) + 1` exists
/// (interpolation needs its right-hand neighbour); `finish()` flushes the
/// tail, where the reference clamps the neighbour to the last frame.
pub struct StreamResampler {
    src_rate: u32,
    dst_rate: u32,
    /// Interleaved source frames not yet consumable (the pending tail).
    buf: Vec<f32>,
    /// Source frames dropped from the front of `buf` so far (global offset).
    consumed: usize,
    /// Next output frame index (global).
    next_out: usize,
}

impl StreamResampler {
    pub fn new(src_rate: u32, dst_rate: u32) -> Self {
        Self { src_rate, dst_rate, buf: Vec::new(), consumed: 0, next_out: 0 }
    }

    /// Feed source-rate frames; get back every output frame now computable.
    pub fn push(&mut self, chunk: &[f32]) -> Vec<f32> {
        if self.src_rate == self.dst_rate {
            return chunk.to_vec();
        }
        self.buf.extend_from_slice(chunk);
        self.emit(false)
    }

    /// The stream is over: emit the tail with the reference's end-clamping.
    pub fn finish(&mut self) -> Vec<f32> {
        if self.src_rate == self.dst_rate {
            return Vec::new();
        }
        self.emit(true)
    }

    fn emit(&mut self, at_end: bool) -> Vec<f32> {
        let ratio = self.dst_rate as f64 / self.src_rate as f64;
        let avail = self.consumed + self.buf.len() / 2; // global frames present
        // Total output frames only becomes known at the end (the reference
        // rounds the whole input's length); mid-stream, emit while the
        // interpolation neighbour surely exists.
        let out_end = if at_end {
            ((avail as f64) * ratio).round() as usize
        } else {
            usize::MAX
        };
        let mut out = Vec::new();
        while self.next_out < out_end {
            let src_pos = self.next_out as f64 / ratio;
            let idx = src_pos.floor() as usize;
            // Mid-stream we must not clamp: wait for the neighbour instead.
            if !at_end && idx + 1 >= avail {
                break;
            }
            let frac = (src_pos - idx as f64) as f32;
            let last = avail.saturating_sub(1);
            let i0 = idx.min(last);
            let i1 = (idx + 1).min(last);
            let (Some(a0), Some(a1)) = (self.local(i0), self.local(i1)) else {
                break;
            };
            for ch in 0..2 {
                let a = a0[ch];
                let b = a1[ch];
                out.push(a + (b - a) * frac);
            }
            self.next_out += 1;
        }
        // Drop source frames no future output frame will read. The earliest
        // future read is floor(next_out/ratio); keep one frame before it as
        // the left neighbour can equal it exactly.
        let keep_from = ((self.next_out as f64 / ratio).floor() as usize).min(avail);
        if keep_from > self.consumed {
            let drop_frames = keep_from - self.consumed;
            self.buf.drain(0..(drop_frames * 2).min(self.buf.len()));
            self.consumed = keep_from;
        }
        out
    }

    /// Global frame `i` out of the retained buffer, if still present.
    fn local(&self, i: usize) -> Option<[f32; 2]> {
        let rel = i.checked_sub(self.consumed)?;
        let base = rel * 2;
        (base + 1 < self.buf.len()).then(|| [self.buf[base], self.buf[base + 1]])
    }
}
```

NOTE to implementer: the bit-identity tests are the ground truth — if any test disagrees with this sketch, FIX THE IMPLEMENTATION to match `resample_stereo`'s arithmetic (e.g. rounding of `out_end`, clamping at the tail), not the tests. Iterate until bit-identical.

- [ ] **Step 3: Run** — `cargo test -p hm-audio resample 2>&1 | tail -4` and `cargo test -p hm-audio chunked 2>&1 | tail -4` → all green; `cargo clippy -p hm-audio --all-targets 2>&1 | tail -3` clean.

- [ ] **Step 4: Commit** — `git add crates/hm-audio/src/decode.rs && git commit -m "feat(decode): chunk-safe resampler, bit-identical to the one-shot"`

---

### Task 2: Chunked decode

**Files:**
- Modify: `crates/hm-audio/src/decode.rs` (`decode_format` ~line 80, wrappers `decode_file`/`decode_bytes` ~lines 68-76; tests)

**Interfaces:**
- Produces (used by Task 4):

```rust
pub enum DecodeChunk {
    /// Sent once, right after probe: tags/cover + the stream's sample rate.
    Meta(crate::meta::TrackMeta, u32),
    /// Source-rate interleaved stereo PCM, in order.
    Pcm(Vec<f32>),
}
pub fn decode_format_chunked(
    format: Box<dyn FormatReader>,
    min_chunk_frames: usize,
    sink: &mut dyn FnMut(DecodeChunk) -> bool, // false = abort (teardown)
) -> Result<(), AudioError>
```

- `decode_file`/`decode_bytes` keep their exact signatures, now implemented over the chunked function (collect Meta + concatenate Pcm). `decode_format` (private) is replaced by the chunked version + a private collector.

- [ ] **Step 1: Failing tests**

A WAV is trivial to synthesize in-test (44-byte header + PCM), so the equivalence test needs no fixture file:

```rust
    /// A minimal 16-bit stereo PCM WAV: enough for symphonia to decode.
    fn tiny_wav(frames: usize, rate: u32) -> Vec<u8> {
        let data_len = (frames * 4) as u32;
        let mut w = Vec::new();
        w.extend(b"RIFF");
        w.extend((36 + data_len).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes()); // PCM
        w.extend(2u16.to_le_bytes()); // stereo
        w.extend(rate.to_le_bytes());
        w.extend((rate * 4).to_le_bytes()); // byte rate
        w.extend(4u16.to_le_bytes()); // block align
        w.extend(16u16.to_le_bytes()); // bits
        w.extend(b"data");
        w.extend(data_len.to_le_bytes());
        for i in 0..frames {
            let v = ((i % 97) as i16).wrapping_mul(199);
            w.extend(v.to_le_bytes()); // L
            w.extend((-v).to_le_bytes()); // R
        }
        w
    }

    #[test]
    fn chunked_decode_equals_whole_decode() {
        let wav = tiny_wav(10_000, 44_100);
        let whole = decode_bytes(wav.clone(), Some("wav")).expect("whole decode");

        let format = open_format_bytes(wav, Some("wav")).expect("probe");
        let mut meta = None;
        let mut rate = 0;
        let mut samples = Vec::new();
        decode_format_chunked(format, 1024, &mut |c| {
            match c {
                DecodeChunk::Meta(m, r) => {
                    meta = Some(m);
                    rate = r;
                }
                DecodeChunk::Pcm(pcm) => samples.extend(pcm),
            }
            true
        })
        .expect("chunked decode");

        assert_eq!(rate, whole.sample_rate);
        assert_eq!(samples.len(), whole.samples.len());
        assert!(samples.iter().zip(&whole.samples).all(|(a, b)| a == b), "bit-identical PCM");
    }

    #[test]
    fn chunked_decode_flushes_in_bounded_chunks() {
        let wav = tiny_wav(10_000, 44_100);
        let format = open_format_bytes(wav, Some("wav")).expect("probe");
        let mut sizes = Vec::new();
        decode_format_chunked(format, 1024, &mut |c| {
            if let DecodeChunk::Pcm(p) = c {
                sizes.push(p.len() / 2);
            }
            true
        })
        .unwrap();
        assert!(sizes.len() > 1, "10k frames at min 1024 must arrive in several chunks");
        // Every flush waited for the minimum except the final partial one.
        assert!(sizes[..sizes.len() - 1].iter().all(|&s| s >= 1024));
    }

    #[test]
    fn a_false_from_the_sink_aborts_the_decode() {
        let wav = tiny_wav(50_000, 44_100);
        let format = open_format_bytes(wav, Some("wav")).expect("probe");
        let mut pcm_calls = 0;
        let _ = decode_format_chunked(format, 256, &mut |c| {
            if matches!(c, DecodeChunk::Pcm(_)) {
                pcm_calls += 1;
            }
            pcm_calls < 2 // stop after the second PCM chunk
        });
        assert_eq!(pcm_calls, 2, "the decode must stop when the sink says stop");
    }
```

Run: `cargo test -p hm-audio chunked_decode 2>&1 | tail -4` → COMPILE ERROR.

- [ ] **Step 2: Implement**

Refactor `decode_format`'s loop: after probe + `extract_metadata` + params, immediately `sink(DecodeChunk::Meta(meta, sample_rate))`; accumulate `append_stereo` output into a pending buffer; whenever `pending.len()/2 >= min_chunk_frames`, `sink(DecodeChunk::Pcm(std::mem::take(&mut pending)))` (return early `Ok(())` if the sink returns false, likewise for the Meta send); after the packet loop, flush the final partial chunk. The empty-file error ("file produced no audio") moves to the collector wrapper (`decode_file`/`decode_bytes` still error when total samples are empty; `decode_format_chunked` itself reports what it saw — a zero-PCM stream is the CALLER's judgment there). Keep `is_eof`/`DecodeError` handling identical.

- [ ] **Step 3: Run** — `cargo test -p hm-audio 2>&1 | tail -3 && cargo clippy -p hm-audio --all-targets 2>&1 | tail -3` → whole crate green (110 + new), no warnings. Existing decode/queue tests prove the wrappers' equivalence in practice.

- [ ] **Step 4: Commit** — `git add crates/hm-audio/src/decode.rs && git commit -m "feat(decode): chunked decode with early metadata"`

---

### Task 3: Read-side `Slot` state machine

**Files:**
- Modify: `crates/hm-audio/src/stream_queue.rs` (struct ~line 239, `ready`/`track_len`/`frame` ~384-400, `drain` ~407, `read` ~464, `seek` ~552, eager harness ~590, tests)

**Interfaces:**
- Produces: `enum Slot { Empty, Growing(Vec<f32>), Done(Vec<f32>) }`; channel type becomes `DecodeEvent { Meta { idx, meta }, Chunk { idx, samples }, Done { idx }, Failed { idx }, Reset { idx } }`; `const START_FRAMES_SECS: f32 = 1.0` (frames = `device_rate as usize`).
- Consumes: nothing new; Task 4 rebuilds the worker on this protocol.

- [ ] **Step 1: Interim worker note (do this first, keep the crate green)**

The worker currently sends `DecodedTrack = (idx, samples, meta)`. In THIS task, convert it minimally to the new protocol — still whole-track: on `Ready(samples, meta)` send `Meta`, then one `Chunk` with all samples, then `Done`; on skip/failure send `Failed`. No behavioral change; Task 4 makes it stream. (`LoadAttempt` stays as-is this task.)

- [ ] **Step 2: Failing tests (the correctness core — write ALL of these)**

Extend the eager harness: `eager()` keeps building `Done` slots (existing tests unchanged). Add a builder for growing state and a drain-injection helper:

```rust
    /// An eager source whose slots are given explicitly (Growing/Done mixes).
    fn eager_slots(slots: Vec<Slot>, crossfade_frames: usize) -> StreamQueueSource {
        let count = slots.len();
        StreamQueueSource {
            tracks: slots,
            metas: vec![TrackMeta::default(); count],
            count,
            device_rate: 1,
            crossfade: Arc::new(AtomicU32::new((crossfade_frames as f32).to_bits())),
            index: 0,
            cursor: 0,
            xf_cursor: 0,
            xf_len: 0,
            rx: None,
            want_tx: None,
            running: Arc::new(AtomicBool::new(true)),
            meta_sink: None,
            current_index: None,
            _worker: None,
        }
    }
```

(With `device_rate = 1`, `START_FRAMES` = 1 frame — tests stay tiny. Add a note where START_FRAMES is computed.)

Tests (names + exact behavior; write real assertions over `read()` outputs like the existing tests do):

1. `a_growing_track_below_the_start_gate_buffers_silence` — `Growing` with 0 frames, `device_rate` such that gate > 0 (use `eager_slots` but override `device_rate: 4` → gate 4 frames; 2 frames present): read produces silence with `produced == frames` (buffering, not EOF).
2. `a_growing_track_past_the_start_gate_plays` — Growing(≥gate): read returns its samples.
3. `an_underrun_on_a_growing_track_stalls_without_advancing` — Growing track, cursor reaches len: silence frames, `index` unchanged, `produced` counts (still alive); then append more samples to the slot (direct `tracks[0]` mutation in-test) and confirm playback resumes from the cursor.
4. `a_boundary_advances_only_when_done` — same scenario but slot flipped to `Done`: read advances to the next track exactly like today.
5. `a_crossfade_defers_while_the_current_track_grows` — current Growing (cursor near its len), next Done, crossfade on: NO fade frames appear (silence-stall instead), then flip current to Done → fade begins on the next read.
6. `a_crossfade_into_a_growing_next_with_enough_head_ramps` — current Done, next Growing with ≥ xf frames: fade happens (compare against the existing crossfade test's expectations).
7. `a_crossfade_waits_for_a_growing_next_below_the_fade_width` — current Done, next Growing < xf frames: no fade; boundary behaves as the existing "late lookahead" path (xf_len latch shortens or gapless-advances) — mirror `a_late_lookahead_still_ramps_fully_out`'s structure.
8. `reset_keeps_the_cursor_and_stalls_until_redecoded` — play into a Growing track, apply what `drain` does for `Reset` (empty the buffer), confirm silence at the same cursor; re-append past the cursor → audio resumes at cursor position (not from 0).
9. `a_failed_track_still_skips` — `Done(empty)` skips to the next track (pin against the existing `skips_a_failed_track`).
10. `drain_applies_the_event_protocol` — build a source WITH a channel (mpsc pair injected; no worker thread): send Meta/Chunk/Chunk/Done for idx 0 and Failed for idx 1; after `drain()` assert slot 0 is `Done` with concatenated samples + meta applied, slot 1 is `Done(empty)`; send `Reset{0}` → slot 0 becomes `Growing(empty)`.

Run: `cargo test -p hm-audio stream_queue 2>&1 | tail -4` → COMPILE ERRORS then failures.

- [ ] **Step 3: Implement the state machine**

- `Slot` enum + helpers:
  - `ready(i)` (start gate, current track): `Done(_)` → true; `Growing(s)` → `s.len()/2 >= start_frames(self.device_rate)`; `Empty` → false. Where `fn start_frames(rate: u32) -> usize { rate as usize }` (1s) — with a doc comment naming the 1s choice and the spec.
  - `done(i)`: `matches!(Done(_))`.
  - `track_len(i)`: `Growing(s) | Done(s)` → `s.len()/2`; `Empty` → 0.
  - `frame(i, f)`: same, over either variant's samples.
- `read()` rule changes, minimal and surgical:
  - The `!self.ready(self.index)` early branch is unchanged in effect (Growing-below-gate lands here).
  - `crossfading` condition gains `&& self.done(self.index)` and the next-readiness becomes `self.done(next) || growing_head_at_least(next, xf)`.
  - The `else` (cursor >= cur_len) branch splits: `if !self.done(self.index) { /* underrun: silence, produced += 1, do NOT advance */ }` else boundary-advance exactly as today.
- `drain()` applies `DecodeEvent`: `Meta` → `metas[idx]` (+ if idx == current, `signal_track()` so the early announce reaches the UI); `Chunk` → `Empty→Growing(samples)` or `Growing.extend(samples)` (a Chunk after `Done` is a protocol bug — debug_assert + ignore); `Done` → `Growing(s)→Done(s)`, `Empty→Done(empty)`; `Failed` → `Done(empty)`; `Reset` → `Growing(empty)` (whatever the prior state).
- `advance_window` frees to `Slot::Empty`. `seek`/`position`/`total_frames` need no change beyond compiling against `track_len`.
- Interim worker per Step 1.

- [ ] **Step 4: Run** — `cargo test -p hm-audio 2>&1 | tail -3` → ALL green: every pre-existing gapless/crossfade/skip/retry test unchanged and passing (they now build `Done` slots via the untouched `eager()`), plus the 10 new. `cargo clippy -p hm-audio --all-targets 2>&1 | tail -3` clean.

- [ ] **Step 5: Commit** — `git add crates/hm-audio/src/stream_queue.rs && git commit -m "feat(stream-queue): growing track slots — play on partial PCM"`

---

### Task 4: Worker overlap — stream-decode with tee + fallback

**Files:**
- Modify: `crates/hm-audio/src/stream_queue.rs` (worker in `spawn` ~line 288, `fetch_once` ~142, new `TeeReader`; tests)
- Possibly touch: `crates/hm-audio/src/decode.rs` (a `open_format_stream(reader, ext_hint)` helper wrapping `ReadOnlySource` — mirror `open_format_bytes`)

**Interfaces:**
- Consumes: `decode_format_chunked` + `DecodeChunk` + `StreamResampler` (Tasks 1–2), the `DecodeEvent` protocol (Task 3).

- [ ] **Step 1: Failing tests for the pieces**

1. `TeeReader` unit tests: wraps an inner `Read` + a `File`; every byte read passes through AND lands in the file; bytes-read counter accurate; inner error propagates after partial reads.
2. Fallback decision (pure fn) tests:

```rust
/// What to do when the streaming decode fails.
/// - The body downloaded completely (tee got every declared byte): the spool
///   is a complete file — decode it whole, exactly the old path (container
///   needed seeking, or a mid-stream hiccup symphonia couldn't ride out).
/// - Body incomplete: transport problem — Retry (Reset first if chunks went out).
fn after_stream_failure(bytes_seen: u64, declared: Option<u64>) -> StreamFailure {
    if declared.is_none_or(|n| bytes_seen >= n) {
        StreamFailure::DecodeSpool
    } else {
        StreamFailure::Retry
    }
}
```

   Tests: complete body → `DecodeSpool`; short body → `Retry`; no declared length → `DecodeSpool` (can't prove truncation — same lenient posture as the old `is_none_or` check).
3. Truncation-at-EOF guard test: decode SUCCEEDS (EOF looked clean) but tee saw fewer than declared bytes → don't classify yet: drain the rest of the response first (read-and-discard, not decode) and reclassify against the post-drain count. Drain completes the count — the RIFF-trailing-chunk case, where the decoder's own EOF was clean and the transport was never actually truncated — → trusted `Done`. Drain still comes up short → genuine truncation → `Reset` + `Retry`, never a short `Done`. (Shipped as `finish_or_drain_then_retry(bytes_seen, declared, drain) -> bool`, layered over the simpler `finish_or_retry(bytes_seen, declared) -> bool` predicate it checks both before and after draining — test all three outcomes: complete up front, complete only after draining, still short after draining.)

Run: `cargo test -p hm-audio --no-run 2>&1 | tail -3` → COMPILE ERROR.

- [ ] **Step 2: Restructure the worker**

Shape (keep `load_with_retry` and the attempt classification intact):

- Split `fetch_once`: `open_stream(client, target) -> Opened { resp: Response, declared: Option<u64> } | Retry | Skip` — the status classification and Range/timeout request building move here verbatim; the body copy goes away.
- Per attempt (inside the `load_with_retry` closure — its return type stays `LoadAttempt`, but `Ready(samples, meta)` becomes `Ready(()/* published via events */)`. Simplest honest shape: change `LoadAttempt` to `enum { Published, Skip, Retry }` and let the closure OWN event publishing; `load_with_retry`'s job — attempts/backoff/stop — is unchanged):
  1. `open_stream`; on Opened, create the spool + `TeeReader { inner: resp, file, seen: 0 }`.
  2. Probe + `decode_format_chunked(open_format_stream(tee, ext), CHUNK_FRAMES, sink)` where the sink: first `Meta` → `tx.send(DecodeEvent::Meta{..})` (and remembers the source rate, builds the `StreamResampler`); each `Pcm` → resample → `tx.send(Chunk)`; returns `running.load()` so teardown aborts the decode.
  3. On decode `Ok`: classify with `finish_or_drain_then_retry(seen, declared, drain)`, not the raw byte count alone — it checks `finish_or_retry(seen, declared)` first, and only if that's short does it actually drain the rest of the response (read-and-discard, not decode) and recheck against the post-drain count. Either check passing → flush `resampler.finish()` as a last Chunk, send `Done` → `Published`. This is what makes a container whose decoder reaches a clean EOF before every declared byte is read (e.g. RIFF/WAV trailing metadata after the audio data) resolve as a trusted `Done` instead of a false-positive truncation. Still short after draining → send `Reset` (if any chunk went out) → `Retry`.
  4. On decode `Err`: `after_stream_failure(seen, declared)`:
     - `DecodeSpool`: finish draining the tee to the spool (`std::io::copy(&mut tee, &mut sink_null)` — the tee keeps writing to the file), then decode the spool whole (`decode_file`), resample whole (`resample_stereo`), publish Meta(if not yet)/one Chunk/Done → `Published`. Send `Reset` first if streaming chunks were already published (the whole-file decode re-publishes from zero — without the Reset the track would be doubled).
     - `Retry`: send `Reset` if needed → `Retry`.
  5. Empty PCM at Done-time (nothing decoded, clean EOF, complete body) → `Failed` semantics: send `Failed` → `Published`-with-skip... simplest: send `Failed { idx }` and return `Published` (the slot becomes `Done(empty)` = skip, matching today's `Skip` outcome for an undecodable file).
- After the ladder: if the ladder exits with Skip/exhausted → send `Failed { idx }` (today it sends an empty track — same skip semantics through the new protocol). Ensure EXACTLY ONE terminal event (`Done`/`Failed`) per track per final outcome, and Resets between attempts.
- `CHUNK_FRAMES`: source-rate frames ≈ 1s (e.g. 48_000) — the gate is device-rate ~1s; a source chunk resamples to ≈1s device-rate. Constant with a doc comment.

- [ ] **Step 3: Run everything**

`cargo test -p hm-audio 2>&1 | tail -3` — the FULL suite green: all Task-3 state-machine tests, all pre-existing retry/skip tests (`retry_recovers_a_transient_failure`, `permanent_failure_skips_without_retrying`, `gives_up_after_max_attempts`, `stop_request_aborts_retrying` — these drive the ladder via the closure and MUST still pass; adapt their closures to the new `LoadAttempt` shape if they construct it directly, preserving each test's meaning). `cargo clippy -p hm-audio --all-targets 2>&1 | tail -3` clean.

- [ ] **Step 4: Live smoke (best effort)**

If a signed-in session exists: `cargo test -p hm-ytmusic -- --ignored live_radio 2>&1 | tail -4` (unrelated but cheap sanity that resolution still works) — and note in the report that a real streamed-start listen happens on the release build per the manual checklist.

- [ ] **Step 5: Commit** — `git add crates/hm-audio/src/stream_queue.rs crates/hm-audio/src/decode.rs && git commit -m "feat(stream-queue): decode while downloading — first note in under a second"`

---

### Task 5: Verification + push

- [ ] **Step 1: Gates**

```bash
cargo clippy --workspace --all-targets 2>&1 | tail -3
cargo test -p hm-audio -p hm-ytmusic -p hm-core 2>&1 | grep "test result"
pnpm exec tsc --noEmit 2>&1 | tail -3
pnpm test -- --run 2>&1 | tail -4
pnpm build 2>&1 | tail -2
```

All green (hm-remote = environmental).

- [ ] **Step 2: Push** — `git push -u origin feat/fastload-phase3`

- [ ] **Step 3: Memory** — update `hypemuzik_desktop_fastload.md`: Phase 3 implemented; manual checklist additions: (a) gapless YT queue start latency by ear (~sub-second on warm URL), (b) crossfade still correct at batch boundaries, (c) a deliberately throttled network (Network Link Conditioner) → track stalls mid-play with silence then resumes (no skip, no crash), (d) seek during the first seconds of a still-growing track clamps gracefully.
