# Task 3 Report: Read-side `Slot` state machine (fast-load phase 3)

> Overwrites a stale report left in this slot. The prior content ("Stream-URL
> cache disk persistence", itself layered over two earlier unrelated tasks'
> reports — "Prebuffer/Rebuffer Gate + StreamTuning" and "Deferred-warmup
> scheduler") belonged to `feat/fastload-phase2` / other plan passes, not this
> task. Everything below is the actual implementation done in this session,
> on branch `feat/fastload-phase3`, commit `3d9e0d5`.

## Summary

Replaced `StreamQueueSource`'s decoded-track storage
(`Vec<Option<Vec<f32>>>`, where `None`/`Some(empty)`/`Some(samples)` meant
undecoded/failed/ready) with a `Slot { Empty, Growing(Vec<f32>), Done(Vec<f32>) }`
state machine, so a track can start playing on **partial** PCM once it's
buffered a 1-second head start, rather than waiting for the whole file to
decode. The worker→read-side channel moved from a whole-track
`(idx, samples, meta)` tuple to a `DecodeEvent` protocol (`Meta`, `Chunk`,
`Done`, `Failed`, `Reset`), applied by `drain()`. The worker itself is
interim this task — still whole-track (`Ready` → one `Meta` + one `Chunk` +
`Done`; failure → `Failed`) — Task 4 makes it stream real chunks.

## Files changed

- `/Users/bruno/me/COTE/hypemuzik-desktop/crates/hm-audio/src/stream_queue.rs` — only file touched. 470 insertions / 33 deletions.

## Implementation

- **`Slot` enum** (+ `Debug`) replacing `Option<Vec<f32>>` as `tracks`' element type.
- **`DecodeEvent` enum** replacing the `DecodedTrack`-typed channel (`DecodedTrack` itself is untouched — it's still `load_with_retry`'s return type, pinned by unedited existing tests).
- **`START_FRAMES_SECS: f32 = 1.0`** + `fn start_frames(rate: u32) -> usize` (free fn, doc comment naming the 1s choice/spec) — frames = `rate as f32 * 1.0`, i.e. `rate` at `device_rate` granularity (1 frame at the test harness's `device_rate = 1`).
- **Helpers**: `ready(i)` (`Done(_)`→true; `Growing(s)`→`len/2 >= start_frames`; else false), new `done(i)` (`matches!(Done(_))`), new `growing_head_at_least(i, frames)`, `track_len`/`frame` updated to match `Growing | Done` uniformly.
- **`read()`**: the `!ready(index)` early-continue is unchanged in effect. `crossfading` gained `&& self.done(self.index)`; the next-track trust condition (`next_trusted`) is now `done(next) || growing_head_at_least(next, xf)` instead of the old `ready(next)`. The old single `else` (cursor ≥ len) branch split in two: `cursor < cur_len` (unchanged) → `!done(index)` **underrun** (silence, no advance, `produced` still counts) → `else` **boundary-advance** (unchanged code, now gated strictly on `Done`).
- **`drain()`**: rewritten around `DecodeEvent`. `Meta` applies unconditionally and calls `signal_track()` when `idx == self.index` (early UI announce). `Chunk`: `Empty→Growing(samples)`, `Growing.extend(samples)`, `Done` → `debug_assert!(false, ...)` + ignored (protocol bug, worker kept going past its own end-signal). `Done`: `Growing(s)|Done(s) → Done(s)`, `Empty → Done(Vec::new())`. `Failed` → `Done(Vec::new())`. `Reset` → `Growing(Vec::new())` (cursor untouched — that's the read side's job, not drain's).
- **`advance_window()`** frees to `Slot::Empty` (was `None`).
- **Interim worker** (`spawn`'s inner loop): `load_with_retry`'s `(idx, samples, meta)` is now translated — empty → `Failed{idx}`; non-empty → `Meta{idx,meta}` then `Chunk{idx,samples}` then `Done{idx}`, each `tx.send` chained with `.and_then`, bailing (as before) if the receiver's gone.
- `seek`/`position`/`total_frames` needed **no changes** beyond compiling against the renamed helpers — confirmed by reading them; nothing there matched on the old `Option` shape directly.

## TDD evidence (RED → GREEN)

1. Wrote all 10 new tests (see below) plus `eager_slots` against the **original, unmodified** file (git-stashed my implementation, pasted only the new test block onto HEAD's `stream_queue.rs`).
2. `cargo test -p hm-audio --no-run` → **44 compile errors** (`cannot find type Slot`, `cannot find type DecodeEvent`, etc.) — genuine RED.
3. Restored the full implementation (`git checkout` the RED-only file, `git stash pop`), diffed byte-for-byte against a pre-stash backup copy to confirm no loss.
4. `cargo test -p hm-audio` → **131 passed, 0 failed** (121 pre-existing + 10 new — exact match to the brief's arithmetic). Also ran `cargo test -p hm-audio --lib stream_queue` to confirm all 24 tests in this module (14 pre-existing + 10 new) individually green.
5. `cargo clippy -p hm-audio --all-targets` → 2 warnings found and fixed (not present at first pass):
   - `Reset` variant "never constructed" (true in production code this task — only tests send it; a future streaming/resume worker is the real producer) → `#[allow(dead_code, reason = "wired for a future streaming/resume worker")]` on the variant, with a doc comment explaining why `drain` already handles it.
   - `clippy::field_reassign_with_default` in the new `drain_applies_the_event_protocol` test (`let mut meta = TrackMeta::default(); meta.title = Some(...)`) → converted to struct-literal-with-`..Default::default()`.
   Final `cargo clippy -p hm-audio --all-targets` → clean, 0 warnings.
6. `cargo check --workspace` → clean (confirms `src-tauri`, which calls `StreamQueueSource::spawn`/`StreamResolver`/`StreamTarget`, is unaffected — none of `Slot`/`DecodeEvent` are public).

## The 10 new tests (all in `stream_queue::tests`)

1. `a_growing_track_below_the_start_gate_buffers_silence`
2. `a_growing_track_past_the_start_gate_plays`
3. `an_underrun_on_a_growing_track_stalls_without_advancing`
4. `a_boundary_advances_only_when_done`
5. `a_crossfade_defers_while_the_current_track_grows`
6. `a_crossfade_into_a_growing_next_with_enough_head_ramps`
7. `a_crossfade_waits_for_a_growing_next_below_the_fade_width`
8. `reset_keeps_the_cursor_and_stalls_until_redecoded`
9. `a_failed_track_still_skips`
10. `drain_applies_the_event_protocol`

## Compile-forced harness adjustments to EXISTING (unedited-in-substance) tests

Three lines, in two pre-existing tests, could not compile against the new `Vec<Slot>` type and were mechanically translated (same value, same meaning, zero behavior change — not edits to test logic or assertions):

- `a_late_lookahead_still_ramps_fully_out`: `src.tracks[1] = None;` → `src.tracks[1] = Slot::Empty;`, and `src.tracks[1] = Some(stereo(-1.0, 100));` → `src.tracks[1] = Slot::Done(stereo(-1.0, 100));`.
- `buffers_silence_while_a_track_is_undecoded`: `src.tracks[0] = None;` → `src.tracks[0] = Slot::Empty;`.

`eager()` itself was rewritten to build `Slot::Done` slots (delegating to the new `eager_slots`), exactly as the brief specified — every existing test that builds tracks through `eager()` needed no change at all.

## Self-review traces

**(a) A `Growing` current track receiving chunks mid-read — no stale-length hazard.**
`read()` calls `self.drain()` exactly once, unconditionally, before the per-frame loop — so any `Chunk`/`Done` events the worker enqueued since the previous `read()` call are applied first. Inside the loop, `cur_len` (and every other track fact — `done`, `ready`, `frame`) is recomputed **per frame** via live accessor calls against `self.tracks[i]`, never cached in a variable that survives across calls. Since nothing inside the loop mutates `tracks` (only `drain()`, which already ran), the value `read()` sees is always drain's latest merge, never a previous call's snapshot. Concretely: call *N* underruns at `cur_len=4`; between calls the worker's `Chunk` grows the buffer to 6 frames; call *N+1* starts with `drain()` applying that chunk, then its first `track_len()` lookup already returns 6 — the two extra frames play immediately rather than one more call being wasted on stale-length silence. This is exactly what test 3 (`an_underrun_on_a_growing_track_stalls_without_advancing`) exercises, just via a direct `tracks[0]` mutation instead of a channel `Chunk` (equivalent from `read()`'s point of view — both are "the slot got longer between calls").

**(b) The `crossfading` boolean can't flip true mid-fade in a way that corrupts the `xf_len` latch while current is `Growing`.**
`crossfading` requires `self.done(self.index)`, recomputed fresh every frame. While the current slot is `Growing`, `done()` is `false` unconditionally — irrespective of `cursor`, `cur_len`, or the next track's readiness — so `crossfading` cannot evaluate `true`, and therefore the `if self.xf_len == 0 { self.xf_len = ... }` latch inside the crossfade branch can never fire. `cur_len` is also meaningless as "distance from the end" while `Growing` (it's just "how much has arrived so far", not the track's true length), which is exactly why the gate exists. Only once `drain()` promotes the slot to `Done` (a `Growing(s)|Done(s) → Done(s)` move, never a partial/in-place rewrite) does `cur_len` become the track's real, final, and now-immutable length (a `Chunk` after `Done` is a protocol-bug no-op, `debug_assert`ed) — and only then can `crossfading` go `true` and the latch compute a width against a length that will not change further. Verified directly by test 5: through the entire Growing/underrun phase (even though `cursor + xf >= cur_len` is already numerically true), `xf_len` stays unlatched and the output is silence, not a fade; the very next read after flipping to `Done` shows the latch firing fresh (`t=0` → full current-track gain, then ramping down), not a corrupted or partial one.
One edge case flagged rather than silently handled: nothing in this task's interim worker (or any other current caller) ever sends `Reset` for the **currently-playing** index mid-fade — `Reset` is presently a `drain()`-side protocol capability with no live producer for that scenario (it's exercised only by direct test harness manipulation). If it ever did happen, `done(index)` would flip back `false` on the next frame, `crossfading` would evaluate `false` despite `xf_len` still being nonzero from before, and playback would fall through to the plain `cursor < cur_len` / underrun branches — silently reverting to un-blended current-track audio (or silence) while `position()`/`total_frames()` (keyed only on `xf_len > 0`) kept reporting the incoming track's clock until the boundary code (unreached in that state) would normally zero it. Not exercised by anything wired in this task; called out per the brief's "report BLOCKED on ambiguity" instruction rather than inventing a fix for a scenario the spec doesn't describe.

**(c) `Done(empty)` (skip) still advances instantly at cursor 0.**
`ready(i)` returns `true` for **any** `Done(_)`, including an empty one — so a failed/skipped track never gets stuck behind the "still buffering" early-continue. With `cur_len = track_len(i) = 0`, the very first frame evaluated for that index takes: `cursor(0) < cur_len(0)` → false, `!done(index)` → false (it *is* `Done`), so it falls straight into the boundary-advance branch on frame 0 — identical to the pre-refactor `Some(Vec::new())` path. Test 9 (`a_failed_track_still_skips`) pins this: first audible sample comes from track 1, at cursor 0, with track 0 producing zero real frames.

**(d) `Meta` for the CURRENT index triggers `signal_track()` so the early announce reaches the UI.**
`drain()`'s `Meta` arm does `self.metas[idx] = meta;` unconditionally, then `if idx == self.index { self.signal_track(); }`. `signal_track()` → `signal_index(self.index)`, which stores the index into `current_index` (if wired) and, if `meta_sink` is wired, calls `sink.set(self.metas[self.index].clone())` — reading the value **just written** in the same `drain()` call, not a stale one. So tag metadata (title/artist/album/cover) can reach the now-playing UI the moment the worker parses it, without waiting for the rest of the file to decode or for a `Done` event — the exact "early announce" the brief calls for. `Meta` for the *lookahead* index (`idx == self.index + 1`) intentionally does **not** call `signal_track()` here; it's surfaced later, either by the crossfade's own `signal_index(self.index + 1)` at the fade's start, or by the boundary-advance's `signal_track()` once that index becomes current — matching the existing "announce when audible" rule the crossfade tests already pin (`crossfade_announces_the_incoming_track_at_its_start`). This code path is exercised (not panicking, meta applied correctly) by test 10, though no test asserts `current_index`/`meta_sink` specifically for `Meta` — the brief's Test 10 spec only checked slot state + `metas`, so no extra test was added beyond what was asked.

## Commit

`3d9e0d5` (original) → **amended to `7903488`** after the review fix below — `feat(stream-queue): growing track slots — play on partial PCM`, same message, no `Co-Authored-By`, **not pushed**.

---

## Post-review fix (opus reviewer, 4 items — all applied)

The reviewer approved the state machine but required one Important fix plus
three authorized hardening items before Task 4 wires a real `Reset`
producer. All four applied, then `3d9e0d5` amended in place (same message).

**1 (Important) — stale fade-latch on aborted crossfades.**
(a) `drain()`'s `Reset` arm now zeroes `xf_len`/`xf_cursor` whenever the
reset touches either fade participant (`idx == self.index || idx ==
self.index + 1`) while a fade is in progress (`xf_len > 0`) — a reset
invalidates whatever length/position the ramp was latched against (the
current track's real length, or the incoming track's buffered head), so
resuming the ramp against it would blend against stale positions in tracks
that no longer hold what they held. Killing the ramp lets the ordinary
underrun/deferred-fade paths take back over, as if the fade had never
started. (b) The gapless-boundary branch of `read()` now also zeroes
`xf_cursor` next to its existing `xf_len = 0` — this was a **pre-existing**
gap (predates this task): a seek/slider-driven jump straight to the boundary
path (bypassing the crossfade branch's own reset) left `xf_cursor` non-zero,
so a *later* fade could in principle start counting from a stale non-zero
`xf_cursor` instead of 0. Both fixes are defensive; no test in this crate
currently exercises either path deeply enough to have caught the gap
(no `Reset`-during-fade producer exists yet, and no existing test drives a
mid-fade seek), which is exactly why the reviewer flagged it ahead of Task 4
wiring a real `Reset` producer.

**2 — post-fade start-gate dropout seam.** Raised the crossfade trust
threshold for a `Growing` next track from `growing_head_at_least(next, xf)`
to `growing_head_at_least(next, xf.max(start_frames(self.device_rate)))`.
Rationale (now in the code as a comment): fading into a track whose buffered
head clears the *fade width* but not the *start gate* would have the
boundary's own `ready()` check fail immediately after the crossfade
completes, cutting already-audible sound to silence — worse than the
deferred-fade path this mechanism exists to avoid. Trusting only a head that
already satisfies the start gate keeps audibility monotone across the
boundary.
*Interaction with existing tests, checked as instructed:* tests 6/7 (and
the pre-existing crossfade tests) all run at `device_rate = 1` via
`eager`/`eager_slots`, so `start_frames(1) == 1`, which never exceeds any
realistic `xf` — `xf.max(1) == xf` in every case. **No existing test's
buffered-head numbers sit between the old and new threshold, so no test
needed updating.** Reran `cargo test -p hm-audio --lib stream_queue` after
the change: all 24 (now 25) tests green, identical pass/fail set.

**3 — Meta re-announce guard.** `drain()`'s `Meta` arm now signals only
`if idx == self.index && self.xf_len == 0`, with a comment: mid-fade, the
incoming track was already announced at the fade's start
(`signal_index(self.index + 1)` in `read`), so re-announcing the *outgoing*
one on a fresh `Meta` would wrong-foot the UI back to it until the next real
change. `self.metas[idx]` is still updated unconditionally either way —
only the `signal_track()` call is gated.

**4 — pin the early announce.** Added
`meta_reaches_the_sink_early_but_not_mid_fade`: builds a source with a real
injected channel *and* a real `MetaSink` + `current_index` (not `None`,
unlike every other harness-built test in this file), sends `Meta{idx: 0}`
before the track is even playable (`Slot::Empty`), drains, and asserts the
sink's read-back handle actually saw the title (not just that `metas[0]`
was set internally, and not just that nothing panicked). Then sets
`xf_len = 4` to simulate being mid-fade, sends a second, different `Meta`
for the same (now-outgoing) index, drains, and asserts `metas[0]` still
updates internally but the **sink does not** — pinning fix #3's guard.

*Enabling change (not in the reviewer's four, but required to write test
4):* `MetaSink`'s fields are private to `engine.rs` and it had no public
constructor — no existing test anywhere in the crate builds a real one
(every `meta_sink` in every harness, in both `stream_queue.rs` and
`queue.rs`, is `None`). Added a `#[cfg(test)] impl MetaSink { pub(crate) fn
for_test() -> (Self, Arc<ArcSwap<TrackMeta>>) }` in `engine.rs`, mirroring
the crate's existing `RadioStreamSource::for_test()` pattern in
`streaming.rs` — returns a sink paired with the same read-side
`Arc<ArcSwap<TrackMeta>>` handle a real caller gets via
`AudioEngine::track_meta_handle`, so the test can assert what was actually
published. `pub(crate)` (not test-file-local) because the constructor lives
in `engine.rs` but is consumed from `stream_queue.rs`'s test module.

### Verification after the fix

- `cargo test -p hm-audio` → **132 passed, 0 failed** (was 131; +1 for the new pinning test). `--lib stream_queue` alone: 25 passed.
- `cargo clippy -p hm-audio --all-targets` → clean, 0 warnings.
- `cargo check --workspace` → clean (confirms `src-tauri` unaffected — `MetaSink::for_test` is `pub(crate)` and `#[cfg(test)]`, invisible outside the crate and outside test builds).

### Files touched by the fix

- `crates/hm-audio/src/stream_queue.rs` — the four behavioral changes + the new test.
- `crates/hm-audio/src/engine.rs` — the `MetaSink::for_test()` test-only constructor (19 lines added, no production-path change).

### Commit

Amended `3d9e0d5` → **`7903488`**, same message
(`feat(stream-queue): growing track slots — play on partial PCM`), no
`Co-Authored-By`, **not pushed**.
