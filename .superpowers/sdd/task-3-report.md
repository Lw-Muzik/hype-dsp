# Task 3 Report: Prebuffer/Rebuffer Gate + StreamTuning

## Commit

`c9aa63e` — feat(streaming): prebuffer/rebuffer gate + configurable 30s ring (StreamTuning)

## Files Changed

- `crates/hm-audio/src/streaming.rs` — Added `StreamTuning` struct + `for_network()`, `should_buffer()` pure fn, two new fields on `RadioStreamSource` (`prebuffer_frames`, `buffering`), updated `new()` to pass `StreamTuning::for_network`, `with_headers()` gains trailing `tuning: StreamTuning` param sizing the ring from `tuning.ring_frames * 2`, gate + re-arm inserted in `read()`, `for_test()` constructor in `#[cfg(test)]` impl block.
- `crates/hm-audio/src/engine.rs` — Updated the single `with_headers` call site (line 1135) to pass `StreamTuning::for_network(sample_rate, false)` as the new 6th argument. The brief said Task 4 would do this, but `engine.rs` is compiled as part of `hm-audio`, so it needed fixing for `-p hm-audio` to compile at all.

## TDD RED/GREEN

RED: Tests `should_buffer_gates_until_prebuffer_then_releases`, `for_network_uses_larger_buffers_in_data_saver`, and `read_holds_silence_until_prebuffered_then_plays` were written first and confirmed failing (compile errors: symbols not yet defined).

GREEN: All 3 new tests pass. Total: 44 tests passing, 0 failing.

## read() alloc-free confirmation

The prebuffer gate path in `read()`:
1. Reads `self.consumer.slots()` — a lock-free atomic load on the ring.
2. Writes silence via `out.iter_mut()` — a simple slice zero-fill.
3. Returns `frames` — no allocation, no locking, no IO.

`should_buffer()` is a pure `fn` over primitive types — zero heap access. The re-arm (`self.buffering = true`) is a plain bool write after the per-frame loop.

## Clippy

`cargo clippy -p hm-audio --all-targets` — clean, no warnings.

## Concerns

None. The engine.rs call-site fix is the minimal bootstrap needed for `-p hm-audio` to compile. Task 4 may extend tuning selection; this change does not conflict with that.
