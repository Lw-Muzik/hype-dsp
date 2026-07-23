# Fast-Load Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Warm/queued YT Music interactions (skip, queue advance, replay, radio append) start in ~0.5–1s instead of 2.5–5s: always-Range on the first progressive open, sequential batch pre-resolution of upcoming tracks, and prefetch spawns deferred off the click.

**Architecture:** Three independent changes: (R1) `open()` in `hm-audio/streaming.rs` always sends `Range` (googlevideo paces plain GETs ~1× realtime); (R2) a `ytmusic_prefetch_batch` command walks video ids sequentially through the existing idempotent `prefetch`, driven from the frontend at queue start and radio append; (R3) a pure `warmup.ts` scheduler defers all prefetch spawns ~3s past the click, latest-wins per key.

**Tech Stack:** Rust (reqwest blocking, hm-audio, hm-ytmusic), Tauri 2, TypeScript + Zustand, vitest (fake timers), cargo test.

**Spec:** `docs/superpowers/specs/2026-07-23-fastload-phase1-design.md`
**Analysis:** `.superpowers/sdd/fable-fastload-analysis.md` (R1–R3 sections)

## Global Constraints

- Branch: `feat/fastload-phase1` (off main, radio feature already merged).
- R1 acceptance rules are exact: `start_byte == 0` accepts **200 or 206**; `start_byte > 0` accepts **206 only** (a 200 would replay from byte 0 — today's behavior, keep it).
- R2 prefetches run **sequentially** (each is a full yt-dlp process; concurrent spawns contend); depth cap **3**; fire-and-forget (a failure costs nothing).
- R3 delay is **3000ms**, latest-wins per key: a newer track's warmup cancels the pending one for the same key; skip-spam must never stack timers.
- Nothing on the audio hot path blocks on a prefetch. No behavior change for local/phone/cloud/radio-station sources.
- Repo rules: no `Co-Authored-By`; push only when the controller says (end of plan).
- Run all commands from repo root `~/me/COTE/hypemuzik-desktop`.

---

### Task 1: R1 — always send Range on the progressive open

**Files:**
- Modify: `crates/hm-audio/src/streaming.rs` (`open()` at ~line 873; tests in `mod tests` at ~line 1131)

**Interfaces:**
- Consumes: existing `open(client, url, headers, start_byte)` shape — signature unchanged.
- Produces: same function, new wire behavior. No callers change.

- [ ] **Step 1: Write the failing wire test**

Append to `mod tests` in `streaming.rs`, modeled on `stream_queue.rs`'s `a_fetch_asks_for_the_body_as_a_range` (~line 626 — read it first; it is the template, including the TcpListener + header-capture pattern):

```rust
    /// googlevideo paces a plain GET to ~1× realtime; the same request carrying
    /// a Range serves the same body ~190× faster. The gapless path learned this
    /// (see `stream_queue.rs`); this is the progressive path's copy of the same
    /// lesson — the FIRST open must ask as a range too, because that first open
    /// is the one the listener is waiting on.
    #[test]
    fn the_first_open_asks_for_the_body_as_a_range() {
        use std::io::{BufRead, BufWriter, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));

        let server_seen = seen.clone();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line == "\r\n" || line == "\n" {
                    break;
                }
                server_seen.lock().unwrap().push(line.trim_end().to_string());
            }
            let body = b"bytes";
            let mut w = BufWriter::new(stream);
            let _ = write!(
                w,
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-4/5\r\n\
                 Content-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = w.write_all(body);
            let _ = w.flush();
        });

        let client = reqwest::blocking::Client::new();
        let r = open(
            &client,
            &format!("http://{addr}/track"),
            &[("User-Agent".into(), "hm-test".into())],
            0,
        );
        server.join().expect("server thread");
        assert!(r.is_some(), "a 206 at byte 0 must be accepted");

        let lines = seen.lock().unwrap().clone();
        let range = lines
            .iter()
            .find(|l| l.to_ascii_lowercase().starts_with("range:"))
            .unwrap_or_else(|| panic!("no Range header was sent; got {lines:#?}"));
        assert!(
            range.eq_ignore_ascii_case("range: bytes=0-"),
            "asked for the wrong range: {range}"
        );
    }

    /// Some radio/Icecast servers ignore Range and answer 200 — at byte 0
    /// that is exactly the body we asked for, so it must keep working.
    #[test]
    fn a_200_at_byte_zero_is_still_accepted() {
        use std::io::{BufRead, BufWriter, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line == "\r\n" || line == "\n" {
                    break;
                }
            }
            let body = b"bytes";
            let mut w = BufWriter::new(stream);
            let _ = write!(
                w,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = w.write_all(body);
            let _ = w.flush();
        });

        let client = reqwest::blocking::Client::new();
        let r = open(&client, &format!("http://{addr}/live"), &[], 0);
        server.join().expect("server thread");
        assert!(r.is_some(), "a Range-ignoring server must not break byte-0 opens");
    }

    /// At an offset a 200 means the server would replay from byte 0 —
    /// audible duplication. That rejection must survive this change.
    #[test]
    fn a_200_at_an_offset_is_still_rejected() {
        use std::io::{BufRead, BufWriter, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line == "\r\n" || line == "\n" {
                    break;
                }
            }
            let body = b"bytes";
            let mut w = BufWriter::new(stream);
            let _ = write!(
                w,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = w.write_all(body);
            let _ = w.flush();
        });

        let client = reqwest::blocking::Client::new();
        let r = open(&client, &format!("http://{addr}/track"), &[], 4096);
        server.join().expect("server thread");
        assert!(r.is_none(), "a replay-from-zero response must be treated as a failed open");
    }
```

(If `Mutex`/`Arc` are already in scope in the test module, drop the redundant paths; match the module's existing imports.)

- [ ] **Step 2: Run to verify the new first test fails**

Run: `cargo test -p hm-audio the_first_open 2>&1 | tail -5`
Expected: FAIL — panic "no Range header was sent" (today `start_byte == 0` sends none). The other two new tests should pass already (they pin current behavior).

- [ ] **Step 3: Change `open()`**

Replace the body of `open()` (~line 873):

```rust
/// Issue the GET, always as a range request from `start_byte`.
///
/// Always ranged, even from byte 0: googlevideo paces a plain GET to about
/// the bitrate of the content — reasonable for a dumb player, ruinous for a
/// buffer trying to get ahead of the decoder — while the same request with a
/// `Range` header is served at full speed (the gapless path measured 190×;
/// see `stream_queue.rs`). Servers that ignore Range answer 200 with the
/// whole body, which at byte 0 is exactly what was asked for.
fn open(
    client: &reqwest::blocking::Client,
    url: &str,
    headers: &[(String, String)],
    start_byte: u64,
) -> Option<reqwest::blocking::Response> {
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    req = req.header("Range", format!("bytes={start_byte}-"));
    match req.send() {
        Ok(r) if start_byte > 0 => {
            // A ranged resume MUST come back as 206; a 200 means the server
            // ignored the Range and would replay the whole body from byte 0
            // (audible duplication + inflated byte count). Treat that as a
            // failed open so the bounded reconnect ladder handles it.
            (r.status() == reqwest::StatusCode::PARTIAL_CONTENT).then_some(r)
        }
        // At byte 0 both a 206 (ranged) and a 200 (Range-ignoring server —
        // internet radio, some CDNs) deliver the body from the start.
        Ok(r) if r.status().is_success() => Some(r),
        _ => None,
    }
}
```

- [ ] **Step 4: Run the crate suite**

Run: `cargo test -p hm-audio 2>&1 | tail -3` and `cargo clippy -p hm-audio --all-targets 2>&1 | tail -3`
Expected: all green (104 + 3 new), zero new warnings. Watch specifically for existing streaming tests that asserted a plain GET — if any fail, they were pinning the bug; update them to expect the Range header and say so in the report.

- [ ] **Step 5: Commit**

```bash
git add crates/hm-audio/src/streaming.rs
git commit -m "fix(streaming): ask for the first byte-range too — plain GETs are paced"
```

---

### Task 2: R2 backend — sequential `ytmusic_prefetch_batch`

**Files:**
- Modify: `crates/hm-ytmusic/src/lib.rs` (next to `prefetch`, ~line 711)
- Modify: `src-tauri/src/commands/ytmusic.rs` (next to `ytmusic_prefetch`, ~line 447)
- Modify: `src-tauri/src/lib.rs` (register beside `commands::ytmusic::ytmusic_prefetch`)
- Modify: `src/lib/ipc.ts` (next to `ytmusicPrefetch`, ~line 413)

**Interfaces:**
- Consumes: existing `YtMusicState::prefetch(&self, video_id) -> Result<(), String>` (idempotent against the URL cache).
- Produces: `YtMusicState::prefetch_batch(&self, video_ids: &[String])`; command `ytmusic_prefetch_batch(video_ids: Vec<String>)`; TS `ytmusicPrefetchBatch(videoIds: string[]): Promise<void>` — used by Task 3.

- [ ] **Step 1: State method**

After `prefetch` in `crates/hm-ytmusic/src/lib.rs`:

```rust
    /// Warm several tracks' stream urls, one at a time.
    ///
    /// Sequential on purpose: each miss is a full yt-dlp process, and two
    /// spawns visibly contend for the CPU and network the click's own resolve
    /// is using. Cache hits cost nothing ([`Self::prefetch`] checks first), so
    /// a caller can re-send ids freely. Same fire-and-forget contract as
    /// `prefetch`: a failure costs nothing because the play path resolves for
    /// itself and reports properly.
    pub fn prefetch_batch(&self, video_ids: &[String]) {
        for id in video_ids {
            let _ = self.prefetch(id);
        }
    }
```

- [ ] **Step 2: Command + registration + wrapper**

`src-tauri/src/commands/ytmusic.rs`, after `ytmusic_prefetch`:

```rust
/// Warm several upcoming tracks' urls (sequentially — see `prefetch_batch`).
/// Fire-and-forget like [`ytmusic_prefetch`]; the play path never waits on it.
// `(async)`: shells out to yt-dlp and waits on the network.
#[tauri::command(async)]
pub fn ytmusic_prefetch_batch(state: State<'_, YtMusicState>, video_ids: Vec<String>) {
    state.prefetch_batch(&video_ids);
}
```

Register `commands::ytmusic::ytmusic_prefetch_batch,` in `src-tauri/src/lib.rs` beside `ytmusic_prefetch`.

`src/lib/ipc.ts`, after `ytmusicPrefetch`:

```ts
/** Warm several upcoming tracks' stream urls (sequential in the backend).
 *  Fire-and-forget; the play path never waits on it. */
export function ytmusicPrefetchBatch(videoIds: string[]): Promise<void> {
  return invoke<void>("ytmusic_prefetch_batch", { videoIds });
}
```

- [ ] **Step 3: Verify**

Run: `cargo check --workspace 2>&1 | tail -3 && cargo clippy -p hm-ytmusic --all-targets 2>&1 | tail -3 && pnpm exec tsc --noEmit 2>&1 | tail -3`
Expected: all clean.

- [ ] **Step 4: Commit**

```bash
git add crates/hm-ytmusic/src/lib.rs src-tauri/src/commands/ytmusic.rs src-tauri/src/lib.rs src/lib/ipc.ts
git commit -m "feat(ytmusic): sequential batch prefetch of upcoming stream urls"
```

---

### Task 3: R3 + R2 frontend — deferred warmups off the click

**Files:**
- Create: `src/stores/warmup.ts`
- Create: `src/stores/warmup.test.ts`
- Modify: `src/stores/engine.ts` (ytmusic branch of `startPlayback` ~lines 585-607; `fetchRadio`'s `.then`; `stop`)

**Interfaces:**
- Consumes: `ytmusicPrefetch`, `ytmusicVideoPrefetch`, `ytmusicPrefetchBatch` (Task 2) from `@/lib/ipc`.
- Produces: `scheduleWarmup(key: string, delayMs: number, fn: () => void): void` and `cancelWarmups(): void` from `@/stores/warmup`.

- [ ] **Step 1: Write the failing tests**

`src/stores/warmup.test.ts`:

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cancelWarmups, scheduleWarmup } from "@/stores/warmup";

describe("scheduleWarmup", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    cancelWarmups();
    vi.useRealTimers();
  });

  it("runs the work only after the delay", () => {
    const fn = vi.fn();
    scheduleWarmup("video", 3000, fn);
    vi.advanceTimersByTime(2999);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("latest wins per key — skip-spam never stacks spawns", () => {
    const a = vi.fn();
    const b = vi.fn();
    scheduleWarmup("video", 3000, a);
    vi.advanceTimersByTime(1500);
    scheduleWarmup("video", 3000, b);
    vi.advanceTimersByTime(3000);
    expect(a).not.toHaveBeenCalled();
    expect(b).toHaveBeenCalledTimes(1);
  });

  it("keys are independent — the video warmup does not cancel the batch", () => {
    const video = vi.fn();
    const batch = vi.fn();
    scheduleWarmup("video", 3000, video);
    scheduleWarmup("batch", 3000, batch);
    vi.advanceTimersByTime(3000);
    expect(video).toHaveBeenCalledTimes(1);
    expect(batch).toHaveBeenCalledTimes(1);
  });

  it("cancelWarmups drops everything pending", () => {
    const fn = vi.fn();
    scheduleWarmup("video", 3000, fn);
    cancelWarmups();
    vi.advanceTimersByTime(10000);
    expect(fn).not.toHaveBeenCalled();
  });

  it("a fired warmup does not linger — rescheduling after it fires works", () => {
    const fn = vi.fn();
    scheduleWarmup("video", 1000, fn);
    vi.advanceTimersByTime(1000);
    scheduleWarmup("video", 1000, fn);
    vi.advanceTimersByTime(1000);
    expect(fn).toHaveBeenCalledTimes(2);
  });
});
```

Run: `pnpm test -- --run src/stores/warmup.test.ts 2>&1 | tail -4` → FAIL (module not found).

- [ ] **Step 2: Implement `warmup.ts`**

```ts
/**
 * Deferred, latest-wins scheduling for prefetch spawns.
 *
 * A click on a track can fire several yt-dlp processes (audio resolve, video
 * warmup, next-track warmup) that all contend for the CPU and network at the
 * exact moment the listener is waiting for sound. Deferring the optional ones
 * a few seconds costs nothing — tracks run minutes — and "latest wins per
 * key" means skip-spam replaces pending work instead of stacking it.
 */

const pending = new Map<string, number>();

/** Run `fn` after `delayMs`, replacing any pending work under the same key. */
export function scheduleWarmup(key: string, delayMs: number, fn: () => void): void {
  const prior = pending.get(key);
  if (prior != null) window.clearTimeout(prior);
  pending.set(
    key,
    window.setTimeout(() => {
      pending.delete(key);
      fn();
    }, delayMs),
  );
}

/** Drop everything pending (playback stopped — nothing is worth warming). */
export function cancelWarmups(): void {
  for (const id of pending.values()) window.clearTimeout(id);
  pending.clear();
}
```

Run: `pnpm test -- --run src/stores/warmup.test.ts 2>&1 | tail -4` → 5 passed.

- [ ] **Step 3: Wire into `engine.ts`**

Imports: add `ytmusicPrefetchBatch` to the `@/lib/ipc` block and `import { cancelWarmups, scheduleWarmup } from "@/stores/warmup";`.

Add next to the other module constants:

```ts
  /** How long after a track starts before optional prefetches may spawn. */
  const WARMUP_DELAY_MS = 3000;
  /** How many upcoming tracks to pre-resolve (sequentially, in the backend). */
  const WARMUP_DEPTH = 3;
```

In `startPlayback`'s ytmusic branch, replace the immediate video prefetch (~line 592):

```ts
        if (item.ytTrack!.hasVideo) {
          // Deferred: the video url matters when the Video tab opens, not in
          // the seconds the audio resolve is fighting for the network.
          const vid = item.ytTrack!.videoId;
          scheduleWarmup("video", WARMUP_DELAY_MS, () => {
            void ytmusicVideoPrefetch(vid).catch(() => {});
          });
        }
```

In the engine-gapless arm (right after `void playerPlayYtmusicQueue(items, pos).catch(onError);`), add:

```ts
          // Warm the next few tracks once this one is safely playing, so a
          // skip lands on a resolved url instead of a ~5s yt-dlp spawn.
          const ahead = order
            .slice(pos + 1, pos + 1 + WARMUP_DEPTH)
            .map((i) => queue[i]?.ytTrack?.videoId)
            .filter((v): v is string => typeof v === "string");
          if (ahead.length > 0) {
            scheduleWarmup("batch", WARMUP_DELAY_MS, () => {
              void ytmusicPrefetchBatch(ahead).catch(() => {});
            });
          }
```

In the single-track arm, replace the immediate next-track prefetch (~lines 599-606):

```ts
          // Resolve the next tracks while this one plays — deferred so the
          // spawns never contend with this track's own resolve (which is
          // what the listener is actually waiting on).
          const aheadSingle: string[] = [];
          let p = pos;
          for (let n = 0; n < WARMUP_DEPTH; n++) {
            const np = stepOrder(p, order.length, repeat, 1);
            if (np === null) break;
            const vid = queue[order[np]!]?.ytTrack?.videoId;
            if (vid) aheadSingle.push(vid);
            p = np;
          }
          if (aheadSingle.length > 0) {
            scheduleWarmup("batch", WARMUP_DELAY_MS, () => {
              void ytmusicPrefetchBatch(aheadSingle).catch(() => {});
            });
          }
```

In `fetchRadio`'s `.then`, after `appendQueueItems(fresh.map(radioItem));` and before `resumeIfEndedNaturally();`:

```ts
        // Warm the first appended tracks: the queue seam otherwise pays a
        // full cold resolve the moment playback crosses into the new batch.
        const warm = fresh.slice(0, WARMUP_DEPTH).map((t) => t.videoId);
        if (warm.length > 0) {
          scheduleWarmup("radio-batch", WARMUP_DELAY_MS, () => {
            void ytmusicPrefetchBatch(warm).catch(() => {});
          });
        }
```

In the `stop` action, next to `resetRadioSession();`, add `cancelWarmups();` (an explicit stop means nothing upcoming is worth warming).

- [ ] **Step 4: Verify**

Run: `pnpm exec tsc --noEmit 2>&1 | tail -3 && pnpm test -- --run 2>&1 | tail -4 && pnpm build 2>&1 | tail -2`
Expected: clean; 132 tests (127 + 5); build clean.

- [ ] **Step 5: Commit**

```bash
git add src/stores/warmup.ts src/stores/warmup.test.ts src/stores/engine.ts
git commit -m "perf(player): defer and batch prefetch spawns off the click"
```

---

### Task 4: Verification + push

- [ ] **Step 1: Whole-workspace gates**

```bash
cargo clippy --workspace --all-targets 2>&1 | tail -3
cargo test -p hm-audio -p hm-ytmusic -p hm-core 2>&1 | grep "test result"
pnpm exec tsc --noEmit 2>&1 | tail -3
pnpm test -- --run 2>&1 | tail -4
pnpm build 2>&1 | tail -2
```

Expected: zero new warnings, all green. (`hm-remote` iroh tests are network-environmental — skip unless the network changed.)

- [ ] **Step 2: Push**

```bash
git push -u origin feat/fastload-phase1
```

- [ ] **Step 3: Update memory**

Update `~/.claude/projects/-Users-bruno-me-COTE/memory/hypemuzik_desktop_fastload.md`: Phase 1 implemented on `feat/fastload-phase1` (commits + what shipped); note NOT device-tested and that the R1 win (2.5-3.5s → ~0.5-1s warm progressive starts) needs a release-build listen test.
