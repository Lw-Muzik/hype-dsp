# Task 2 Report: Byte-counting reader + wire resume into the worker

## Status
COMPLETE — commit `30f6e32` on branch `feat/crossfade-cloud-phone`, pushed to remote.

## TDD Evidence

### RED phase
Before adding the test + struct, `cargo test -p hm-audio counting_reader` returned 0 tests found (filtered out) — no such test existed.

### GREEN phase
Added `CountingReader<R>` struct + `impl std::io::Read for CountingReader<R>` immediately, then added `counting_reader_counts_bytes_read` test.
`cargo test -p hm-audio counting_reader` → PASSED.

## Files Changed
- `crates/hm-audio/src/streaming.rs` (sole file, 76 insertions / 19 deletions)

### Changes
1. **`CountingReader<R>`** — added after `impl Drop for RadioStreamSource`. Wraps inner reader, tallies bytes read via `Arc<AtomicU64>` (`Ordering::Relaxed`).

2. **`decode_connection` signature** — new `conn_bytes: Arc<AtomicU64>` second parameter. Response is wrapped in `CountingReader { inner: response, count: conn_bytes }` before being fed to `ReadOnlySource::new` → `MediaSourceStream`. All other function body is unchanged.

3. **`stream_worker` reconnect loop** — replaced old 2-state loop with full 5-state loop per brief:
   - `stalls`, `conn_bytes` (shared `Arc`), `connect_fails` added.
   - `conn_bytes.store(0)` resets before each connection attempt.
   - `open()` failures retry up to `MAX_STALLS` times with 400ms×n back-off, then fall back to byte 0, then give up.
   - `progressed` + `consumed` computed from `conn_bytes` after each `decode_connection`.
   - `Stop::Eof` → `resume_decision(total, consumed, progressed, stalls)`:
     - `ResumeDecision::Resume` → update `start_byte`/`stalls`, sleep 300ms, `continue` (no `finished` set).
     - `ResumeDecision::Finish` → `finished = true`, enter idle seek-wait loop (unchanged).
   - `Stop::Seek` resets `stalls = 0`.

4. **Test** — `counting_reader_counts_bytes_read` added to the existing `#[cfg(test)] mod tests`.

## Test Summary
`cargo test -p hm-audio` → **41 passed; 0 failed**. All prior tests (id3v1, byte_offset, to_stereo, resume_decision) continue to pass.

## Clippy State
`cargo clippy -p hm-audio --all-targets` → **zero warnings**. The 3 Task-1 dead_code warnings (`resume_decision`, `ResumeDecision`, `MAX_STALLS`) are eliminated — all three are now actively used in the wired-up `stream_worker` loop.

## Self-Review
- `conn_bytes` uses `Ordering::Relaxed` throughout — correct; both `store` and `load` happen sequentially on the worker thread only.
- `consumed = start_byte + conn_bytes.load()` correctly accounts for accumulated byte range offsets across reconnects.
- Idle seek-wait loop is entered only on `ResumeDecision::Finish`, preserving prior EOF behaviour exactly.
- `connect_fails` resets to 0 on each successful open so accumulated failures don't bleed across stable connections.
- `Instant`/`sleep` calls are on the worker thread — the RT `AudioSource::read()` path is unaffected.

## Concerns
None. Implementation matches the brief verbatim.
