# Task 2 report: wire the native resolver into `YtMusicState::resolve` + telemetry + live canary

(This overwrites a stale prior version of this report — that older version covered
an unrelated Task 2 from the fast-load **Phase 3** plan, chunked decode. This is the
current, correct report for Task 2 of the fast-load **Phase 4** plan.)

## Branch / commit

- Branch: `feat/fastload-phase4`
- Commit: `6d22f7f feat(ytmusic): resolve natively first, yt-dlp as the floor`
  (amended in place after reviewer follow-up — see "Post-review follow-up" below;
  original pre-follow-up SHA was `a123531`, same message, no Co-Authored-By, not pushed)
- On top of Task 1's `e47e92d feat(ytmusic): native InnerTube player resolution — the fast path`
- Not pushed (per instructions).
- Files changed:
  - `crates/hm-ytmusic/src/lib.rs` (+124/-1 net, across several insertion points, after the amend)
  - `crates/hm-ytmusic/src/innertube.rs` (-1: removed the `#[allow(dead_code)]` on `resolve_native`)

## What changed

### Step 1 — Counters + gate

- `YtMusicState` gained two fields: `native_hits: AtomicU64`, `native_misses: AtomicU64`,
  both initialized to 0 in `YtMusicState::new()`.
- Module-level `fn native_resolve_enabled() -> bool`, gated by `HM_NATIVE_RESOLVE=0`
  via a `OnceLock<bool>` (read once per process), placed next to `probe_client`/`probe_ok`.
- `fn native_miss(&self, reason: &str)` method (next to `remember_target`): bumps
  `native_misses`, reads the current `native_hits`, and emits one `eprintln!` per miss
  with the reason and the running tally. Hits stay silent (no log line), per spec.

### Step 2 — The `resolve` integration

Wired in verbatim, between the existing cache check (`live_or_probed_target`) and the
existing `let runner = self.runner()?;` yt-dlp path — nothing after that line was
touched:

```rust
if native_resolve_enabled() {
    match innertube::resolve_native(probe_client(), video_id) {
        Ok(target) if probe_ok(&target) => {
            self.native_hits.fetch_add(1, Ordering::Relaxed);
            self.remember_target(video_id, &target);
            return Ok(target);
        }
        Ok(_) => self.native_miss("probe refused the url"),
        Err(miss) => self.native_miss(&miss.to_string()),
    }
}
```

A probed-OK native target goes through the same `remember_target` the yt-dlp path
uses, so Phase 2's persistence (`resolved` map → disk snapshot) and Phase 1's prefetch
inherit it automatically — no separate wiring needed. A native miss (either
`resolve_native` erroring, or `probe_ok` refusing the url) falls straight into the
pre-existing yt-dlp `resolve_with_fallback` call, completely unchanged.

Also removed the `#[allow(dead_code)]` on `innertube::resolve_native` (Task 1 had it
marked "consumed by Task 2") — zero dead-code allowances remain in the crate.

### Step 3 — The live throttle canary

Added `live_native_resolve_is_fast_and_unthrottled` verbatim from the brief, placed
next to the other `--ignored` live async tests (right after `live_radio_pages_endlessly`,
before the `fresh_in` helper). No adjustment was needed beyond what the brief already
anticipated — it compiled as given. It needs no keychain (`android_vr` is anonymous),
so it doesn't use the `live_state()` skip-visibility helper the keychain-bound tests use;
it's a pure network test, `#[tokio::test]` + `#[ignore = "requires network access"]`,
matching the tokio pattern of the crate's other live async tests.

## Post-review follow-up

Reviewer approved but flagged a real gap: the `resolve()` integration seam itself
(gate → native → probe → remember vs. fall-through) had zero coverage — even the
canary called `innertube::resolve_native` directly, bypassing `resolve()` entirely.
Closed it by extending `live_native_resolve_is_fast_and_unthrottled` (same test,
same `#[ignore]` reason — no new test added) with a second half that exercises the
real seam on a fresh `YtMusicState`:

1. Builds `Arc::new(YtMusicState::new())` — no keychain needed (native is anonymous;
   cookies/session play no role in the fast path).
2. Calls `state.resolve("dQw4w9WgXcQ")` via `spawn_blocking` and asserts:
   - `Ok` with `format_id == "140"`.
   - `state.native_hits == 1` (with a message calling out client-constant rot if
     this is 0 instead — i.e. yt-dlp answered instead of native).
   - `state.native_misses == 0`.
3. Calls `state.resolve(...)` a **second** time for the same id and asserts:
   - The url matches the first call's (cache round-trip).
   - `native_hits` is *still* 1 and `native_misses` is *still* 0 — proving the
     cache check in `resolve()` short-circuits before the native attempt is ever
     retried (if the cache had missed, one of the two counters would necessarily
     have moved, since every native attempt increments exactly one of them).

This directly proves the seam the reviewer named: the gate is live, native wins on
a fresh state, a probed-OK native target is remembered (that's *why* the second
call is a cache hit at all — `remember_target` is what populated it), and the
cache-first order in `resolve()` is real, not assumed.

Also took the **Minor**: the 64KB-range-fetch assertion previously read
`"the resolved url must serve ranged bytes"` with no byte count in the message,
even though the code asserts a 32KB floor against a 64KB request. Chose to **keep
the 32KB floor** (not raise it to the full 65,536) — more robust to a CDN chunking
the response slightly short of the full requested range on an otherwise-healthy,
unthrottled connection — and reworded the message to name it explicitly:
`"the resolved url must serve at least 32KB of the 64KB requested range"`.

### Re-verification after the follow-up

```
cargo test -p hm-ytmusic 2>&1 | tail -3
```
→ `test result: ok. 139 passed; 0 failed; 16 ignored; 0 measured; 0 filtered out`
(same 139/16 as before — the follow-up extends the existing canary test in place,
it doesn't add a new `#[test]`/`#[tokio::test]` item, so the ignored-test count is
unchanged.)

```
cargo clippy -p hm-ytmusic --all-targets 2>&1 | tail -3
```
→ `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 1.93s` — clean.

```
cargo check --workspace 2>&1 | tail -10
```
→ Clean (same pre-existing, unrelated `block` crate future-incompat notice as before).

**Live canary re-run**, exact requested command:
```
cargo test -p hm-ytmusic -- --ignored live_native 2>&1 | tail -6
```
```
   Doc-tests hm_ytmusic

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```
(the unittest binary's own summary line scrolls past a plain `tail -6` because the
doc-test phase runs after it and prints its own 0-test summary — same shape as any
`cargo test` invocation on this crate.) The actual result, with `--nocapture` to
surface the timing `eprintln!`:
```
running 1 test
native 64KB fetch: 55.028958ms
test tests::live_native_resolve_is_fast_and_unthrottled ... ok
```
**PASSED**, including the new seam assertions (silent on success — a failure in any
of `native_hits`/`native_misses`/cache-url-match would have printed the panic
message and failed the test, which it did not). Re-ran the direct-resolve_native
half's timing a third time for a spot check: `55ms`/`56ms`/`52ms` across three
separate runs earlier in this task — all comfortably under the 2s threshold, no
sign of throttling.

## Verification evidence (original Steps 1–4, pre-follow-up)

```
cargo test -p hm-ytmusic 2>&1 | tail -3
```
→ `test result: ok. 139 passed; 0 failed; 16 ignored; 0 measured; 0 filtered out`

```
cargo clippy -p hm-ytmusic --all-targets 2>&1 | tail -3
```
→ Initially 2 warnings (`clippy::doc_lazy_continuation` — a doc-comment line on the new
`native_hits`/`native_misses` field started with `+`, which clippy's markdown parser
read as a lazy list continuation). Fixed by rewording that one comment line (`+ one
probe` → `and a probe`). Re-run: `Finished \`dev\` profile ... ` — zero warnings.

```
cargo check --workspace 2>&1 | tail -3
```
→ Clean (one pre-existing, unrelated future-incompat notice from the `block` crate
dependency — not from this change).

### THE CANARY — live run (the timing line)

Per-brief exact command:
```
cargo test -p hm-ytmusic -- --ignored 2>&1 | tail -10
```
```
failures:
    tests::live_a_playlist_from_search_opens_into_tracks
    tests::live_a_song_from_search_opens_into_itself
    tests::live_an_artist_from_search_opens_into_a_page_with_tracks
    tests::live_playlists_load_completely
    tests::live_radio_pages_endlessly

test result: FAILED. 11 passed; 5 failed; 0 ignored; 0 measured; 139 filtered out; finished in 48.34s
```

**`live_native_resolve_is_fast_and_unthrottled` was among the 11 that PASSED in this
run.** The 5 failures are pre-existing, unrelated `#[tokio::test]` live tests (search,
playlists, radio) that panic with `"error sending request for url ... received."` —
transport-level failures, not assertion failures, and not from anything this task
touched. Confirmed this is default-parallelism (16 concurrent tests, all hammering
`music.youtube.com`'s API from the same signed-in session/IP simultaneously) network
contention, not a regression:

```
cargo test -p hm-ytmusic -- --ignored --test-threads=1 2>&1 | tail -10
```
→ `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 139 filtered out; finished in 80.17s`
— every single live test, including the new canary, passes when run without concurrent
contention. Which 5 fail under parallelism is non-deterministic between runs (verified:
a second parallel run failed a different 5-test subset, `live_explore_items_open_into_tracks`
included that time but not the isolated set) — consistent with the crate's own existing
doc comment about concurrent keychain/API contention flaking live tests that pass alone.

**The canary's timing, isolated (`--test-threads=1 --nocapture`):**
```
test tests::live_native_resolve_is_fast_and_unthrottled ... native 64KB fetch: 52.150083ms
ok
```
Re-run standalone again (fresh process, under default parallel load this time) for a
second data point:
```
native 64KB fetch: 306.8265ms
test tests::live_native_resolve_is_fast_and_unthrottled ... ok
```
Both comfortably under the 2s throttle-detection threshold — confirms the
`ANDROID_VR` client constants (`CLIENT_VERSION = "1.62.27"`) are current, itag 140
resolves natively, and the resulting url serves at real CDN speed (no SABR/n-param
throttling). **No STOP condition triggered — shipping the fast path is warranted.**

## Self-review (per task instructions' checklist)

1. **No added latency when native is disabled** — `native_resolve_enabled()` is
   checked first (`if native_resolve_enabled() { ... }`); when `HM_NATIVE_RESOLVE=0`,
   the whole block is skipped and `resolve` falls straight to `self.runner()?` exactly
   as before Task 2. The `OnceLock` env read happens once per process, not per call.

2. **No lock held around the native attempt** — inspected `resolve`'s body: the only
   things acquiring a lock before/around the `innertube::resolve_native(...)` call are
   `live_or_probed_target` (returns before the fast-path block, guard already dropped)
   and, on the hit path, `self.remember_target` (acquires/releases its own `RwLock`
   internally, no outer guard spans the network call). The native POST and the probe
   GET both run with zero locks held.

3. **`resolve`'s error type/messages unchanged for the fallback path** — the code
   after the new block (`let runner = self.runner()?; ... resolve_with_fallback(...)`)
   is byte-identical to what Task 1 left; a native miss only calls `self.native_miss`
   (logs + counts, returns nothing) and then falls through to that same pre-existing
   code, so every existing error string/type a caller could see is untouched.

4. **`prefetch`/`prefetch_batch` inherit the fast path — verified, not assumed:**
   ```rust
   pub fn prefetch(&self, video_id: &str) -> Result<(), String> {
       if self.live_or_probed_target(video_id).is_some() {
           return Ok(());
       }
       self.resolve(video_id).map(|_| ())
   }

   pub fn prefetch_batch(&self, video_ids: &[String]) {
       for id in video_ids {
           let _ = self.prefetch(id);
       }
   }
   ```
   Both route through `resolve` with no separate resolve logic of their own, so both
   gain the native fast path automatically.

5. **Formatting note (non-issue, same pattern as Phase 3 Task 2's report):**
   `cargo fmt -p hm-ytmusic -- --check` shows 68 diffs post-change vs 66 pre-change
   (crate was already not fully rustfmt-clean before this task, unrelated to this
   change). Of the 2 new diffs, both are inside the canary test body the brief
   mandated be added **verbatim** (`req.send().map(...).unwrap_or(false)` and the final
   `assert!` — both exceed rustfmt's line width as written in the brief). Confirmed
   every line of *implementation* code added in this task (counters, gate,
   `native_miss`, the `resolve` integration) introduces zero fmt diffs of its own.

## Concerns / follow-ups for later tasks

- The 5 flaky pre-existing live test failures under default parallelism (search/
  playlists/radio, all keychain-gated) are not new and not caused by this task, but
  are worth flagging: running the full `--ignored` suite in CI or by hand may need
  `--test-threads=1` (or per-test filtering) for a trustworthy read, since concurrent
  hits against one signed-in session appear to get transport-level errors from
  YouTube's API under load.
- `native_hits`/`native_misses` are counted but not yet surfaced anywhere (no
  `YtMusicStatus` field, no UI). That's outside this task's scope (brief only asked
  for counters + the log line) but is the natural next hook if a status surface is
  wanted later.
