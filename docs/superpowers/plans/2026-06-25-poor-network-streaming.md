# Poor-network (2G/3G) Streaming Robustness — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make cloud/phone/radio streaming start fast and keep playing on slow/flaky links (2G/3G) by streaming progressively, resuming dropped connections instead of ending the track, prebuffering, and adaptively choosing gapless vs. progressive — with a Data Saver override.

**Architecture:** `RadioStreamSource` (progressive, ring-buffered) becomes the robust workhorse: it resumes on connection drops via HTTP `Range`, gates playback behind a prebuffer, and meters throughput. A Data Saver flag + a session network classifier in the store choose between the gapless `StreamQueueSource` (fast links) and progressive single-track playback (slow/Data-Saver).

**Tech Stack:** Rust (`hm-audio`, `hm-core`, `src-tauri`), `reqwest` blocking, `symphonia`, `rtrb` ring buffer; React/Zustand/TypeScript frontend.

## Global Constraints

- Rust toolchain: stable (currently 1.96); `cargo clippy` must stay clean (CI gates on it across 3 OSes).
- Real-time audio callback (`Renderer::render` → `AudioSource::read`) must not allocate, lock, block, or do I/O. Atomics only.
- `std::time::Instant` is allowed on worker threads only — never in `read()` (RT path).
- `PlaybackState` is `#[serde(rename_all = "camelCase", default)]`; new fields must have `#[serde(default)]`-compatible defaults so saved state still loads.
- No `Co-Authored-By` trailers in commits. Push after the final commit of each phase.
- Frontend dropdowns/toggles follow existing `SettingsView.tsx` patterns.

---

## File structure

| File | Responsibility | Tasks |
|------|----------------|-------|
| `crates/hm-audio/src/streaming.rs` | Progressive source: resume-on-drop, prebuffer gate, tuning, metering | 1,2,3,7 |
| `crates/hm-audio/src/lib.rs` | `AudioSource::buffering()` default | 8 |
| `crates/hm-audio/src/engine.rs` | `PlaybackPos` buffering/bps fields; pass tuning + data_saver; `set_data_saver`; render writes buffering | 4,5,8 |
| `crates/hm-core/src/types.rs` | `PlaybackState.data_saver` | 5 |
| `src-tauri/src/commands/engine.rs` | `engine_set_data_saver` command | 5 |
| `src-tauri/src/lib.rs` | forward `buffering`/`downloadBps`/`rebufferCount` on `engine:progress` | 8 |
| `src/lib/types.ts`, `src/lib/ipc.ts` | TS `PlaybackState.dataSaver`, progress fields, IPC setter | 5,6,9 |
| `src/stores/engine.ts` | Data Saver setter; session network classifier; adaptive `startPlayback` | 6,9 |
| `src/features/settings/SettingsView.tsx` | Data Saver toggle UI | 6 |
| `src/components/NowPlayingBar.tsx` | "Buffering…" indicator | 10 |

---

# PHASE 1 — Progressive robustness (makes 2G/3G work)

## Task 1: Resume-vs-finish decision (pure function)

**Files:**
- Modify: `crates/hm-audio/src/streaming.rs` (add near the `Stop` enum, ~line 243)
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `enum ResumeDecision { Finish, Resume { offset: u64, stalls: u32 } }`, `const MAX_STALLS: u32 = 3`, `fn resume_decision(content_bytes: u64, consumed: u64, progressed: bool, stalls: u32) -> ResumeDecision`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn resume_decision_distinguishes_drop_from_end() {
    // Known length, not all consumed, made progress → resume from the offset.
    assert!(matches!(
        resume_decision(1000, 400, true, 0),
        ResumeDecision::Resume { offset: 400, stalls: 0 }
    ));
    // Reached the end → finish.
    assert!(matches!(resume_decision(1000, 1000, true, 0), ResumeDecision::Finish));
    // Unknown length (radio / no content-length) → finish (unchanged behaviour).
    assert!(matches!(resume_decision(0, 12345, false, 0), ResumeDecision::Finish));
    // A stalled reconnect (no progress) increments the counter…
    assert!(matches!(
        resume_decision(1000, 400, false, 1),
        ResumeDecision::Resume { offset: 400, stalls: 2 }
    ));
    // …until it exceeds the cap, then give up (finish so the queue advances).
    assert!(matches!(resume_decision(1000, 400, false, MAX_STALLS), ResumeDecision::Finish));
    // Progress resets the stall counter even at a high prior count.
    assert!(matches!(
        resume_decision(1000, 700, true, MAX_STALLS),
        ResumeDecision::Resume { offset: 700, stalls: 0 }
    ));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p hm-audio resume_decision -- --nocapture`
Expected: FAIL — `cannot find function resume_decision`.

- [ ] **Step 3: Write minimal implementation**

```rust
/// How many no-progress reconnects in a row we tolerate before giving up on a
/// track (so a server that keeps closing, or a container we can't re-probe
/// mid-file, ends the track instead of hot-looping).
const MAX_STALLS: u32 = 3;

/// What to do after a connection's decode loop stops.
#[derive(Debug, PartialEq, Eq)]
enum ResumeDecision {
    /// The track is genuinely done (reached the end, unknown length, or we gave
    /// up after too many stalls) — report EOF.
    Finish,
    /// The connection dropped early — re-open with `Range: bytes=offset-`.
    Resume { offset: u64, stalls: u32 },
}

/// Decide whether a stopped connection is a real end or a recoverable drop.
/// `consumed` = total bytes read so far; `progressed` = did the just-ended
/// connection read new bytes; `stalls` = prior consecutive no-progress count.
fn resume_decision(content_bytes: u64, consumed: u64, progressed: bool, stalls: u32) -> ResumeDecision {
    // Unknown length (live/radio) or the whole body consumed → genuine end.
    if content_bytes == 0 || consumed >= content_bytes {
        return ResumeDecision::Finish;
    }
    let stalls = if progressed { 0 } else { stalls + 1 };
    if stalls > MAX_STALLS {
        ResumeDecision::Finish
    } else {
        ResumeDecision::Resume { offset: consumed, stalls }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p hm-audio resume_decision`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/hm-audio/src/streaming.rs
git commit -m "feat(streaming): resume-vs-finish decision for dropped connections"
```

---

## Task 2: Byte-counting reader + wire resume into the worker

**Files:**
- Modify: `crates/hm-audio/src/streaming.rs` — add `CountingReader`; change `decode_connection` signature; rework `stream_worker` loop (~lines 247-333, 538-549)
- Test: same file

**Interfaces:**
- Consumes: `resume_decision`, `MAX_STALLS` (Task 1)
- Produces: `struct CountingReader<R> { inner: R, count: Arc<AtomicU64> }` impl `std::io::Read`; `decode_connection(..., conn_bytes: Arc<AtomicU64>, ...)` now wraps its reader in `CountingReader`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn counting_reader_counts_bytes_read() {
    use std::io::Read;
    let count = Arc::new(AtomicU64::new(0));
    let data = vec![1u8, 2, 3, 4, 5, 6, 7];
    let mut r = CountingReader { inner: std::io::Cursor::new(data), count: count.clone() };
    let mut buf = [0u8; 4];
    assert_eq!(r.read(&mut buf).unwrap(), 4);
    assert_eq!(count.load(Ordering::Relaxed), 4);
    let mut rest = Vec::new();
    r.read_to_end(&mut rest).unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 7, "counts every byte read");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p hm-audio counting_reader`
Expected: FAIL — `cannot find struct CountingReader`.

- [ ] **Step 3: Add `CountingReader`**

Add below the `RadioStreamSource` impl block:

```rust
/// Wraps a reader and tallies bytes successfully read, so the worker knows how
/// far it got before a connection dropped (for `Range`-based resume).
struct CountingReader<R> {
    inner: R,
    count: Arc<AtomicU64>,
}

impl<R: std::io::Read> std::io::Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}
```

- [ ] **Step 4: Run the unit test to verify it passes**

Run: `cargo test -p hm-audio counting_reader`
Expected: PASS.

- [ ] **Step 5: Thread the counter through `decode_connection`**

Change its signature (add `conn_bytes`) and wrap the reader. Replace the first line of `decode_connection`:

```rust
fn decode_connection(
    response: reqwest::blocking::Response,
    conn_bytes: Arc<AtomicU64>,
    device_rate: u32,
    producer: &mut Producer<f32>,
    shared: &StreamShared,
    meta_sink: Option<crate::engine::MetaSink>,
    duration_hint: Option<f64>,
    start_byte: u64,
    ext: Option<&str>,
    meta_published: &mut bool,
) -> Stop {
    let counted = CountingReader { inner: response, count: conn_bytes };
    let mss = MediaSourceStream::new(Box::new(ReadOnlySource::new(counted)), Default::default());
    // ...rest unchanged...
```

- [ ] **Step 6: Rework the `stream_worker` reconnect loop**

Replace the worker's `loop { ... }` body (the open → decode → match block) with:

```rust
    let mut start_byte = 0u64;
    let mut meta_published = false;
    let mut stalls = 0u32;
    let conn_bytes = Arc::new(AtomicU64::new(0));
    let mut connect_fails = 0u32;

    loop {
        if !shared.running.load(Ordering::Relaxed) {
            return;
        }
        conn_bytes.store(0, Ordering::Relaxed);

        let Some(response) = open(&client, url, headers, start_byte) else {
            // Couldn't (re)open. Retry a few times (2G connect is slow/flaky),
            // then fall back to the start once, then give up.
            connect_fails += 1;
            if connect_fails <= MAX_STALLS {
                std::thread::sleep(Duration::from_millis(400 * connect_fails as u64));
                continue;
            }
            if start_byte > 0 {
                start_byte = 0;
                connect_fails = 0;
                continue;
            }
            return;
        };
        connect_fails = 0;
        record_content_length(&shared, &response, start_byte);

        let sink = if meta_published { None } else { meta_sink.clone() };
        let stop = decode_connection(
            response,
            conn_bytes.clone(),
            device_rate,
            &mut producer,
            &shared,
            sink,
            duration_hint,
            start_byte,
            ext.as_deref(),
            &mut meta_published,
        );

        let progressed = conn_bytes.load(Ordering::Relaxed) > 0;
        let consumed = start_byte + conn_bytes.load(Ordering::Relaxed);

        match stop {
            Stop::Cancelled => return,
            Stop::Seek(target) => {
                start_byte = byte_offset(&shared, device_rate, target);
                shared.finished.store(false, Ordering::Relaxed);
                stalls = 0;
            }
            Stop::Eof => {
                let total = shared.content_bytes.load(Ordering::Relaxed);
                match resume_decision(total, consumed, progressed, stalls) {
                    ResumeDecision::Resume { offset, stalls: s } => {
                        // Connection dropped mid-track — resume, don't end it.
                        start_byte = offset;
                        stalls = s;
                        shared.finished.store(false, Ordering::Relaxed);
                        std::thread::sleep(Duration::from_millis(300));
                        continue;
                    }
                    ResumeDecision::Finish => {
                        shared.finished.store(true, Ordering::Relaxed);
                        // Idle: a finished-but-seekable stream can still be scrubbed.
                        loop {
                            if !shared.running.load(Ordering::Relaxed) {
                                return;
                            }
                            let target = shared.seek_target.swap(-1, Ordering::Relaxed);
                            if target >= 0 {
                                start_byte = byte_offset(&shared, device_rate, target as u64);
                                shared.finished.store(false, Ordering::Relaxed);
                                stalls = 0;
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(25));
                        }
                    }
                }
            }
        }
    }
```

- [ ] **Step 7: Run the crate tests + clippy**

Run: `cargo test -p hm-audio && cargo clippy -p hm-audio --all-targets`
Expected: PASS, no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/hm-audio/src/streaming.rs
git commit -m "feat(streaming): resume dropped connections via HTTP Range instead of ending the track"
```

---

## Task 3: Prebuffer + rebuffer gate, configurable ring (`StreamTuning`)

**Files:**
- Modify: `crates/hm-audio/src/streaming.rs` — `StreamTuning`, `RadioStreamSource` fields, `with_headers`, `read()`, test constructor
- Test: same file

**Interfaces:**
- Produces: `pub struct StreamTuning { pub prebuffer_frames: usize, pub ring_frames: usize }` + `StreamTuning::for_network(device_rate: u32, data_saver: bool) -> Self`; `should_buffer(available_frames, prebuffer_frames, finished, buffering) -> bool`; `RadioStreamSource::with_headers(url, headers, device_rate, meta_sink, duration_hint, tuning)` (new trailing `tuning` arg).
- Consumed by: Task 4 (engine call site), Task 8 (`buffering()` reads `self.buffering`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn should_buffer_gates_until_prebuffer_then_releases() {
    // While buffering: hold until we have the cushion (or finished).
    assert!(should_buffer(1, 4, false, true), "below target → keep buffering");
    assert!(!should_buffer(4, 4, false, true), "met target → release");
    assert!(!should_buffer(0, 4, true, true), "finished → release even if short");
    // Once playing, the gate is open (underrun handled inside the read loop).
    assert!(!should_buffer(0, 4, false, false));
}

#[test]
fn for_network_uses_larger_buffers_in_data_saver() {
    let normal = StreamTuning::for_network(48_000, false);
    let saver = StreamTuning::for_network(48_000, true);
    assert!(saver.prebuffer_frames > normal.prebuffer_frames);
    assert!(saver.ring_frames >= normal.ring_frames);
    assert!(normal.prebuffer_frames > 0 && normal.ring_frames > normal.prebuffer_frames);
}

#[test]
fn read_holds_silence_until_prebuffered_then_plays() {
    // 4-frame prebuffer; push 2 → buffering (silence, produced>0, not EOF).
    let (mut prod, src_consumer) = RingBuffer::<f32>::new(64);
    for _ in 0..2 { prod.push(0.5).unwrap(); prod.push(0.5).unwrap(); }
    let mut src = RadioStreamSource::for_test(src_consumer, 4);
    let mut out = vec![0.0f32; 6]; // 3 frames
    assert_eq!(src.read(&mut out, 2), 3, "buffering counts as produced (not EOF)");
    assert!(out.iter().all(|&s| s == 0.0), "silence while buffering");
    // Top up past the target → it releases and plays real audio.
    for _ in 0..4 { prod.push(0.5).unwrap(); prod.push(0.5).unwrap(); }
    let mut out2 = vec![0.0f32; 4];
    src.read(&mut out2, 2);
    assert_eq!(out2[0], 0.5, "plays buffered audio once the cushion is met");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p hm-audio -- should_buffer for_network read_holds`
Expected: FAIL — `StreamTuning` / `should_buffer` / `for_test` not found.

- [ ] **Step 3: Add `StreamTuning` + `should_buffer`**

```rust
/// Buffering/ring sizing for a stream, derived from the network mode. Larger on
/// constrained links so a slow download builds a cushion instead of stuttering.
#[derive(Clone, Copy, Debug)]
pub struct StreamTuning {
    /// Frames the ring must hold before (re)starting playback.
    pub prebuffer_frames: usize,
    /// Ring capacity in frames (stereo → 2× this many samples).
    pub ring_frames: usize,
}

impl StreamTuning {
    /// Default tuning for a device rate, picking bigger buffers in Data Saver.
    pub fn for_network(device_rate: u32, data_saver: bool) -> Self {
        let rate = device_rate.max(8_000) as usize;
        let prebuffer_secs = if data_saver { 6 } else { 2 };
        let ring_secs = if data_saver { 45 } else { 30 };
        Self {
            prebuffer_frames: rate * prebuffer_secs,
            ring_frames: rate * ring_secs,
        }
    }
}

/// Whether playback should stay gated (emit buffering silence) this block.
/// Once playing (`buffering == false`) the gate is open; an empty ring is
/// handled inside the read loop, which re-arms `buffering` on a full drain.
fn should_buffer(available_frames: usize, prebuffer_frames: usize, finished: bool, buffering: bool) -> bool {
    buffering && !(finished || available_frames >= prebuffer_frames)
}
```

- [ ] **Step 4: Add fields to `RadioStreamSource`**

In the struct add:

```rust
    /// Target cushion (frames) before (re)starting playback.
    prebuffer_frames: usize,
    /// True while we're holding for the cushion to fill (start or after a drain).
    buffering: bool,
```

- [ ] **Step 5: Update `with_headers` to take `tuning` and size the ring**

Change the signature and the ring sizing:

```rust
    pub fn with_headers(
        url: String,
        headers: Vec<(String, String)>,
        device_rate: u32,
        meta_sink: Option<crate::engine::MetaSink>,
        duration_hint: Option<f64>,
        tuning: StreamTuning,
    ) -> Self {
        let capacity = tuning.ring_frames * 2; // stereo
        let (producer, consumer) = RingBuffer::<f32>::new(capacity);
        // ...existing shared/thread setup unchanged...
        Self {
            consumer,
            shared,
            device_rate,
            prebuffer_frames: tuning.prebuffer_frames,
            buffering: true,
            _thread: thread,
        }
    }
```

Also update `RadioStreamSource::new` (the no-headers helper) to pass `StreamTuning::for_network(device_rate, false)`.

- [ ] **Step 6: Gate `read()` on the prebuffer + re-arm on drain**

At the top of `read()` after the `flushing` early-return, before the per-frame loop:

```rust
        let finished = self.shared.finished.load(Ordering::Relaxed);
        let available = self.consumer.slots() / 2;
        if should_buffer(available, self.prebuffer_frames, finished, self.buffering) {
            for s in out.iter_mut() {
                *s = 0.0;
            }
            return frames; // buffering: silence, counted as produced (not EOF)
        }
        self.buffering = false;
```

(Remove the now-duplicate `let finished = ...` further down.) After the per-frame loop, before `produced` is returned, re-arm on a full drain:

```rust
        if !finished && self.consumer.slots() < 2 {
            self.buffering = true; // ring drained mid-track → rebuffer next block
        }
```

- [ ] **Step 7: Add a test-only constructor**

```rust
#[cfg(test)]
impl RadioStreamSource {
    /// Build a source over a caller-owned ring (no network), for testing the
    /// read/prebuffer gate. `prebuffer_frames` is the cushion under test.
    fn for_test(consumer: rtrb::Consumer<f32>, prebuffer_frames: usize) -> Self {
        let shared = Arc::new(StreamShared {
            running: AtomicBool::new(true),
            position_frames: AtomicU64::new(0),
            total_frames: AtomicU64::new(0),
            content_bytes: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            seek_target: AtomicI64::new(-1),
            flushing: AtomicBool::new(false),
        });
        Self {
            consumer,
            shared,
            device_rate: 1,
            prebuffer_frames,
            buffering: true,
            _thread: std::thread::spawn(|| {}),
        }
    }
}
```

- [ ] **Step 8: Run tests + clippy**

Run: `cargo test -p hm-audio && cargo clippy -p hm-audio --all-targets`
Expected: PASS, no warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/hm-audio/src/streaming.rs
git commit -m "feat(streaming): prebuffer/rebuffer gate + configurable 30s ring (StreamTuning)"
```

---

## Task 4: Engine passes tuning to the stream source

**Files:**
- Modify: `crates/hm-audio/src/engine.rs` — `PlayStream` handler (~line 1135), import `StreamTuning`

**Interfaces:**
- Consumes: `StreamTuning::for_network` (Task 3), `EngineState.data_saver` (Task 5 — until then pass `false`)
- Produces: stream sources built with network-aware tuning.

- [ ] **Step 1: Import `StreamTuning`**

Change `use crate::streaming::RadioStreamSource;` to:

```rust
use crate::streaming::{RadioStreamSource, StreamTuning};
```

- [ ] **Step 2: Build the source with tuning**

In the `PlayStream` handler, read the current Data Saver flag from the live state snapshot and pass tuning:

```rust
                let data_saver = shared.load().playback.data_saver;
                let tuning = StreamTuning::for_network(sample_rate, data_saver);
                let source = Box::new(RadioStreamSource::with_headers(
                    url,
                    headers,
                    sample_rate,
                    Some(sink),
                    duration_hint,
                    tuning,
                ));
```

(`shared` is the `Arc<ArcSwap<EngineState>>` already in scope in `control_loop`. Until Task 5 adds `data_saver`, temporarily pass `false` and revisit — Task 5 adds the field.)

- [ ] **Step 3: Build the workspace**

Run: `cargo check -p hm-audio`
Expected: compiles (after Task 5, `playback.data_saver` resolves).

- [ ] **Step 4: Commit**

```bash
git add crates/hm-audio/src/engine.rs
git commit -m "feat(engine): build stream sources with network-aware StreamTuning"
```

---

## Task 5: `data_saver` flag end-to-end (Rust state + command + TS type)

**Files:**
- Modify: `crates/hm-core/src/types.rs` (`PlaybackState`, ~line 320; `Default`, ~line 328)
- Modify: `crates/hm-audio/src/engine.rs` (add `set_data_saver`)
- Modify: `src-tauri/src/commands/engine.rs` (add `engine_set_data_saver`) and register it in the invoke handler (`src-tauri/src/lib.rs`)
- Modify: `src/lib/types.ts` (TS `PlaybackState`), `src/lib/ipc.ts` (`engineSetDataSaver`)

**Interfaces:**
- Produces: `PlaybackState.data_saver: bool` (serde `dataSaver`); `AudioEngine::set_data_saver(&self, on: bool)`; Tauri `engine_set_data_saver(on: bool)`; TS `engineSetDataSaver(on: boolean): Promise<void>`.

- [ ] **Step 1: Add the Rust field + default**

In `PlaybackState`:

```rust
pub struct PlaybackState {
    /// Play a track list with no silence between tracks.
    pub gapless: bool,
    /// Crossfade duration in seconds (0 = off). Implies gapless when > 0.
    pub crossfade_secs: f32,
    /// Low-bandwidth mode: stream progressively (no full-download / prefetch),
    /// bigger buffers. Forces progressive single-track playback for cloud/phone.
    pub data_saver: bool,
}
```

In its `Default`, add `data_saver: false,`.

- [ ] **Step 2: Add `set_data_saver` to the engine**

After `set_playback`:

```rust
    /// Toggle Data Saver (low-bandwidth) mode. Takes effect on the next stream.
    pub fn set_data_saver(&self, on: bool) {
        self.update(|s| s.playback.data_saver = on);
    }
```

- [ ] **Step 3: Add the Tauri command + register it**

In `src-tauri/src/commands/engine.rs`, mirroring `engine_set_playback`:

```rust
#[tauri::command]
pub fn engine_set_data_saver(engine: State<'_, AudioEngine>, on: bool) {
    engine.set_data_saver(on);
}
```

Add `commands::engine::engine_set_data_saver` to the `tauri::generate_handler![...]` list in `src-tauri/src/lib.rs`.

- [ ] **Step 4: Add the TS type + IPC**

In `src/lib/types.ts` `PlaybackState`, add `dataSaver: boolean;`. In `src/lib/ipc.ts`:

```ts
/** Toggle Data Saver / low-bandwidth streaming mode. */
export function engineSetDataSaver(on: boolean): Promise<void> {
  return invoke<void>("engine_set_data_saver", { on });
}
```

- [ ] **Step 5: Build both sides**

Run: `cargo check -p hypemuzik && pnpm tsc --noEmit`
Expected: both compile. (`pnpm tsc --noEmit` for the TS check; if the project uses `pnpm build`, run that instead.)

- [ ] **Step 6: Commit**

```bash
git add crates/hm-core/src/types.rs crates/hm-audio/src/engine.rs src-tauri/src/commands/engine.rs src-tauri/src/lib.rs src/lib/types.ts src/lib/ipc.ts
git commit -m "feat: Data Saver (low-bandwidth) flag end-to-end"
```

---

## Task 6: Data Saver toggle UI + store wiring (Phase 1 selection)

**Files:**
- Modify: `src/stores/engine.ts` — `defaultEngineState.playback` (~line 142), add `setDataSaver`, gate cloud/phone gapless on `!dataSaver`
- Modify: `src/features/settings/SettingsView.tsx` — toggle row

**Interfaces:**
- Consumes: `engineSetDataSaver` (Task 5)
- Produces: `EngineStore.setDataSaver(on: boolean): void`; `startPlayback` treats `dataSaver` → progressive.

- [ ] **Step 1: Default + import**

In `defaultEngineState.playback`, add `dataSaver: false`. Add `engineSetDataSaver` to the imports from `../lib/ipc`.

- [ ] **Step 2: Add the setter to the store**

Add to the store object (next to `setPlayback`):

```ts
    setDataSaver: (on: boolean) => {
      set((s) => ({ state: { ...s.state, playback: { ...s.state.playback, dataSaver: on } } }));
      void engineSetDataSaver(on).catch(() => {});
    },
```

Add `setDataSaver: (on: boolean) => void;` to the `EngineStore` interface.

- [ ] **Step 3: Gate cloud/phone gapless on Data Saver**

In `startPlayback`, change the `wantQueue` line so Data Saver forces progressive:

```ts
    const { gapless, crossfadeSecs, dataSaver } = state.playback;
    const wantQueue = !dataSaver && (gapless || crossfadeSecs > 0) && repeat !== "one";
```

(Cloud/phone now use the progressive single-track path when Data Saver is on; local gapless is unaffected because it doesn't stream.)

- [ ] **Step 4: Add the toggle to settings**

In `SettingsView.tsx`, in the playback section (near the gapless/crossfade controls), add a row bound to the store:

```tsx
<label className="flex items-center justify-between gap-4">
  <span>
    <span className="font-medium">Data Saver</span>
    <span className="block text-sm text-muted">
      Stream progressively on slow connections (no full-download / prefetch).
    </span>
  </span>
  <input
    type="checkbox"
    checked={playback.dataSaver}
    onChange={(e) => setDataSaver(e.target.checked)}
  />
</label>
```

Wire `const playback = useEngineStore((s) => s.state.playback);` and `const setDataSaver = useEngineStore((s) => s.setDataSaver);` at the top of the component, following the existing gapless/crossfade bindings. Match the file's existing control styling rather than the placeholder classes above.

- [ ] **Step 5: Build the frontend**

Run: `pnpm build` (or `pnpm tsc --noEmit`)
Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add src/stores/engine.ts src/features/settings/SettingsView.tsx
git commit -m "feat(ui): Data Saver toggle — forces progressive cloud/phone streaming"
```

**Phase 1 is now shippable.** Run the full check and push:

```bash
cargo test -p hm-audio && cargo clippy -p hm-audio --all-targets && cargo check -p hypemuzik && pnpm build
git push
```

Manual check: with macOS **Network Link Conditioner** set to "3G", play a cloud track with Data Saver on — it should start within a few seconds and survive a toggled Wi-Fi drop without ending the track.

---

# PHASE 2 — Adaptive auto-detection + buffering UI

## Task 7: Throughput + rebuffer metering in the stream source

**Files:**
- Modify: `crates/hm-audio/src/streaming.rs` — `StreamShared` fields, worker metering, source accessors, `read()` rebuffer counter
- Test: same file

**Interfaces:**
- Produces: `StreamShared.download_bps: AtomicU64`, `StreamShared.rebuffer_count: AtomicU32`; `RadioStreamSource::download_bps(&self) -> u64`, `rebuffer_count(&self) -> u32`, `is_buffering(&self) -> bool`.

- [ ] **Step 1: Add the fields**

In `StreamShared` add `download_bps: AtomicU64` and `rebuffer_count: AtomicU32` (add `use std::sync::atomic::AtomicU32;`). Initialise both to 0 in every `StreamShared { ... }` literal (real ctor, `for_test`, and the test helper `shared(...)`).

- [ ] **Step 2: Write the failing test**

```rust
#[test]
fn read_counts_rebuffer_events() {
    let (mut prod, consumer) = RingBuffer::<f32>::new(64);
    for _ in 0..6 { prod.push(0.5).unwrap(); prod.push(0.5).unwrap(); }
    let mut src = RadioStreamSource::for_test(consumer, 4);
    let mut out = vec![0.0f32; 12]; // drains the 6 frames, then underruns
    src.read(&mut out, 2);
    assert_eq!(src.rebuffer_count(), 1, "draining mid-track arms one rebuffer");
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p hm-audio read_counts_rebuffer`
Expected: FAIL — `rebuffer_count` not found.

- [ ] **Step 4: Increment on rebuffer + add accessors**

In `read()`, change the drain re-arm to count the transition:

```rust
        if !finished && self.consumer.slots() < 2 && !self.buffering {
            self.buffering = true;
            self.shared.rebuffer_count.fetch_add(1, Ordering::Relaxed);
        }
```

Add accessors on `RadioStreamSource`:

```rust
    /// Most recent download throughput estimate, bytes/sec (0 until measured).
    pub fn download_bps(&self) -> u64 {
        self.shared.download_bps.load(Ordering::Relaxed)
    }
    /// Count of mid-track rebuffering events so far this stream.
    pub fn rebuffer_count(&self) -> u32 {
        self.shared.rebuffer_count.load(Ordering::Relaxed)
    }
    /// Whether playback is currently held waiting for the buffer to fill.
    pub fn is_buffering(&self) -> bool {
        self.buffering
    }
```

- [ ] **Step 5: Meter throughput in the worker**

In `push_all` (or around the decode loop), measure bytes/sec via `Instant`. Replace `push_all`'s body to update an EWMA periodically — track a `last = Instant::now()` and a running byte tally in `stream_worker`, and once per ~1s compute `bps` and store it:

```rust
// In stream_worker, before the loop:
let mut meter_start = std::time::Instant::now();
let mut meter_bytes = 0u64;
// After each successful decode_connection progress check (Task 2 `consumed`):
meter_bytes += conn_bytes.load(Ordering::Relaxed);
let elapsed = meter_start.elapsed().as_secs_f64();
if elapsed >= 1.0 {
    let bps = (meter_bytes as f64 / elapsed) as u64;
    let prev = shared.download_bps.load(Ordering::Relaxed);
    // EWMA (3:1) to smooth bursts.
    let smoothed = if prev == 0 { bps } else { (prev * 3 + bps) / 4 };
    shared.download_bps.store(smoothed, Ordering::Relaxed);
    meter_start = std::time::Instant::now();
    meter_bytes = 0;
}
```

(Place this in the worker loop where `consumed`/`progressed` are computed, so it samples each connection's bytes.)

- [ ] **Step 6: Run tests + clippy**

Run: `cargo test -p hm-audio && cargo clippy -p hm-audio --all-targets`
Expected: PASS, no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/hm-audio/src/streaming.rs
git commit -m "feat(streaming): meter download throughput + rebuffer count"
```

---

## Task 8: Publish `buffering` + estimate to the UI

**Files:**
- Modify: `crates/hm-audio/src/lib.rs` — `AudioSource::buffering()` default
- Modify: `crates/hm-audio/src/streaming.rs` — implement `buffering()`/`download_bps()` overrides on the trait
- Modify: `crates/hm-audio/src/engine.rs` — `PlaybackPos` fields + `render()` writes them
- Modify: `src-tauri/src/lib.rs` — include fields in the `engine:progress` payload

**Interfaces:**
- Produces: `AudioSource::buffering(&self) -> bool { false }`, `AudioSource::download_bps(&self) -> u64 { 0 }`, `AudioSource::rebuffer_count(&self) -> u32 { 0 }`; `PlaybackPos` gains `buffering`, `download_bps`, `rebuffer_count` (+ getters/`write_net`); progress event carries `buffering: bool`, `downloadBps: u64`, `rebufferCount: u32`.

- [ ] **Step 1: Add trait defaults**

In `crates/hm-audio/src/lib.rs` `AudioSource`, add:

```rust
    /// Whether the source is currently buffering (holding for the network).
    fn buffering(&self) -> bool {
        false
    }
    /// Latest download throughput estimate, bytes/sec (0 if unknown).
    fn download_bps(&self) -> u64 {
        0
    }
    /// Mid-track rebuffer events so far (0 if not applicable).
    fn rebuffer_count(&self) -> u32 {
        0
    }
```

- [ ] **Step 2: Override on `RadioStreamSource`**

In its `impl AudioSource`, add:

```rust
    fn buffering(&self) -> bool {
        self.is_buffering()
    }
    fn download_bps(&self) -> u64 {
        self.download_bps()
    }
    fn rebuffer_count(&self) -> u32 {
        self.rebuffer_count()
    }
```

- [ ] **Step 3: Add `PlaybackPos` fields + writer**

In `PlaybackPos` add `buffering: AtomicBool`, `download_bps: AtomicU64`, `rebuffer_count: AtomicU32` (init 0/false). Add:

```rust
    fn write_net(&self, buffering: bool, download_bps: u64, rebuffer_count: u32) {
        self.buffering.store(buffering, Ordering::Relaxed);
        self.download_bps.store(download_bps, Ordering::Relaxed);
        self.rebuffer_count.store(rebuffer_count, Ordering::Relaxed);
    }
    pub fn is_buffering(&self) -> bool { self.buffering.load(Ordering::Relaxed) }
    pub fn download_bps(&self) -> u64 { self.download_bps.load(Ordering::Relaxed) }
    pub fn rebuffer_count(&self) -> u32 { self.rebuffer_count.load(Ordering::Relaxed) }
```

In `render()`, after `pos.set_seekable(...)`:

```rust
        pos.write_net(
            self.source.buffering(),
            self.source.download_bps(),
            self.source.rebuffer_count(),
        );
```

- [ ] **Step 4: Extend the progress event**

In `src-tauri/src/lib.rs`, add `buffering: bool`, `download_bps: u64`, `rebuffer_count: u32` to the `Progress` struct (serde `camelCase`), and populate them in the `tick % 6 == 0` emit from `pos.is_buffering()`, `pos.download_bps()`, `pos.rebuffer_count()`.

- [ ] **Step 5: Build**

Run: `cargo check -p hypemuzik && cargo clippy -p hm-audio --all-targets`
Expected: compiles, no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/hm-audio/src/lib.rs crates/hm-audio/src/streaming.rs crates/hm-audio/src/engine.rs src-tauri/src/lib.rs
git commit -m "feat: publish buffering state + download throughput to the UI"
```

---

## Task 9: Session network classifier + adaptive `startPlayback`

**Files:**
- Modify: `src/lib/types.ts` — `TransportProgress` adds `buffering`, `downloadBps`, `rebufferCount`
- Modify: `src/stores/engine.ts` — classifier state + `applyProgress` + `startPlayback` decision
- Test: `src/stores/engine.test.ts` (decision table) — if no test runner exists, extract the decision to `src/stores/networkMode.ts` and test that pure function with the project's test command.

**Interfaces:**
- Consumes: progress fields (Task 8)
- Produces: `type NetworkMode = "unknown" | "fast" | "constrained"`; `chooseStreamMode(source, dataSaver, net): "gapless" | "progressive"`.

- [ ] **Step 1: Write the failing decision test**

Create `src/stores/networkMode.ts` and `src/stores/networkMode.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { chooseStreamMode, classify } from "./networkMode";

describe("chooseStreamMode", () => {
  it("forces progressive under Data Saver", () => {
    expect(chooseStreamMode("cloud", true, "fast")).toBe("progressive");
  });
  it("uses progressive until classified fast", () => {
    expect(chooseStreamMode("cloud", false, "unknown")).toBe("progressive");
    expect(chooseStreamMode("phone", false, "constrained")).toBe("progressive");
  });
  it("upgrades to gapless on a fast link", () => {
    expect(chooseStreamMode("cloud", false, "fast")).toBe("gapless");
  });
});

describe("classify", () => {
  it("marks constrained on any rebuffer", () => {
    expect(classify("fast", { downloadBps: 9_000_000, rebufferDelta: 1 })).toBe("constrained");
  });
  it("marks fast when throughput is comfortably high and stable", () => {
    expect(classify("unknown", { downloadBps: 600_000, rebufferDelta: 0 })).toBe("fast");
  });
  it("stays unknown when throughput is mid and no rebuffer", () => {
    expect(classify("unknown", { downloadBps: 200_000, rebufferDelta: 0 })).toBe("unknown");
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm vitest run src/stores/networkMode.test.ts`
Expected: FAIL — module not found. (If the repo has no vitest, add it dev-only or fold these asserts into an existing test setup; the pure functions are the deliverable.)

- [ ] **Step 3: Implement the pure functions**

```ts
// src/stores/networkMode.ts
export type NetworkMode = "unknown" | "fast" | "constrained";

/** Throughput at/above this (bytes/sec ≈ 4.8 Mbps) comfortably prefetches a
 *  next track during the current one → safe for gapless. */
const FAST_BPS = 400_000;

/** Update the session network classification from one progress sample. A
 *  rebuffer always means constrained; sustained high throughput means fast. */
export function classify(
  prev: NetworkMode,
  sample: { downloadBps: number; rebufferDelta: number },
): NetworkMode {
  if (sample.rebufferDelta > 0) return "constrained";
  if (prev === "constrained") return "constrained"; // sticky until a new queue
  if (sample.downloadBps >= FAST_BPS) return "fast";
  return prev; // not enough evidence yet
}

/** Pick the playback mode for a streamed (cloud/phone) queue. */
export function chooseStreamMode(
  source: "cloud" | "phone",
  dataSaver: boolean,
  net: NetworkMode,
): "gapless" | "progressive" {
  void source;
  if (dataSaver) return "progressive";
  return net === "fast" ? "gapless" : "progressive";
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm vitest run src/stores/networkMode.test.ts`
Expected: PASS.

- [ ] **Step 5: Wire into the store**

In `src/lib/types.ts` `TransportProgress`, add `buffering: boolean; downloadBps: number; rebufferCount: number;`.

In `src/stores/engine.ts`:
- Add module-scoped `let networkMode: NetworkMode = "unknown";` and `let lastRebuffer = 0;` near `gaplessQueueRunning`.
- In `applyProgress`, update the classifier:

```ts
    applyProgress: (p) => {
      const delta = Math.max(0, p.rebufferCount - lastRebuffer);
      lastRebuffer = p.rebufferCount;
      networkMode = classify(networkMode, { downloadBps: p.downloadBps, rebufferDelta: delta });
      set((s) => ({
        positionSecs: p.positionSecs,
        durationSecs: p.durationSecs ?? s.durationSecs,
        paused: p.paused,
        seekable: p.seekable,
        buffering: p.buffering,
      }));
    },
```

(Add `buffering: boolean` to the store state + interface, default `false`; reset `networkMode`/`lastRebuffer` to `"unknown"`/`0` in `setQueueAndPlay` so each new queue re-measures.)

- In `startPlayback`, replace the cloud/phone `useCloudQueue`/`usePhoneQueue` gating to consult `chooseStreamMode`:

```ts
    const streamMode = chooseStreamMode(
      item.source === "phone" ? "phone" : "cloud",
      dataSaver,
      networkMode,
    );
    const wantQueue = (gapless || crossfadeSecs > 0) && repeat !== "one";
    const useCloudQueue =
      item.source === "cloud" && allCloud && wantQueue && streamMode === "gapless";
    const usePhoneQueue =
      item.source === "phone" && allPhone && wantQueue && streamMode === "gapless";
```

Import `classify, chooseStreamMode, NetworkMode` from `./networkMode`.

- [ ] **Step 6: Build**

Run: `pnpm build`
Expected: compiles.

- [ ] **Step 7: Commit**

```bash
git add src/lib/types.ts src/stores/engine.ts src/stores/networkMode.ts src/stores/networkMode.test.ts
git commit -m "feat(ui): adaptive gapless↔progressive by measured network quality"
```

---

## Task 10: "Buffering…" indicator in the now-playing bar

**Files:**
- Modify: `src/components/NowPlayingBar.tsx` (~near the position/duration row, lines 200-214)

**Interfaces:**
- Consumes: store `buffering` (Task 9)

- [ ] **Step 1: Read the flag**

Near the other `useEngineStore` selectors (line ~61):

```tsx
const buffering = useEngineStore((s) => s.buffering);
```

- [ ] **Step 2: Show it**

Where the elapsed time renders (line ~200), show buffering instead of the timestamp while buffering:

```tsx
{buffering ? <span className="text-accent-strong">Buffering…</span> : formatTime(positionSecs)}
```

(Match the file's existing class names/spacing.)

- [ ] **Step 3: Build**

Run: `pnpm build`
Expected: compiles.

- [ ] **Step 4: Commit + push**

```bash
git add src/components/NowPlayingBar.tsx
git commit -m "feat(ui): show Buffering… in the now-playing bar on slow links"
git push
```

---

## Self-review notes

- **Spec coverage:** A1 resume → Tasks 1–2; A2 prebuffer/rebuffer → Task 3; A3 bigger ring → Task 3; A4 metering → Task 7; B engine/IPC (data_saver, tuning, network publish) → Tasks 4,5,8; C frontend (toggle, classifier, adaptive, buffering UI) → Tasks 6,9,10. Phasing matches the spec (1–6 = Phase 1, 7–10 = Phase 2).
- **Type consistency:** `StreamTuning{prebuffer_frames, ring_frames}`, `ResumeDecision::Resume{offset,stalls}`, `should_buffer(...)`, `resume_decision(...)`, `chooseStreamMode/classify`, progress fields `buffering/downloadBps/rebufferCount`, `PlaybackState.data_saver`/`dataSaver` are used identically across tasks.
- **Known follow-ups (not blockers):** radio (unknown-length) drops still finish rather than reconnect — out of scope here (cloud/phone is the target); mid-file resume quality depends on container format (MP3/AAC best), matching the existing seek limitation.
