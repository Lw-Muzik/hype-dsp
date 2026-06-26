# Non-blocking library load (incremental ingestion)

**Date:** 2026-06-26
**Status:** Approved (Approach A)

## Problem

Connecting a drive with 20,000+ songs makes the Library tab **freeze the whole
UI on every open** until loading finishes. The user must wait, unable to
navigate, every single time. Requirement: the app must stay fully interactive
during load even at 1M tracks — "navigate freely without fail".

## Root cause

Opening the Library runs `ensureLocal` → `library_list` IPC, which returns
**every track in one payload**. Three things then block the main thread until
done:

1. `library_list` is a **synchronous** Tauri command
   (`src-tauri/src/commands/library.rs`). Sync commands run on Tauri's main
   thread, so the SQLite read + JSON serialization of all rows blocks there.
   (Same class of bug as the prior lyrics-freeze fix, which moved a sync command
   to `(async)`.)
2. **One giant IPC payload** → the webview must `JSON.parse` a multi-MB blob in a
   single JS task. This is the dominant freeze and is what makes 1M tracks
   impossible (≈200MB+ string).
3. A **synchronous `.map()` over the whole array** (`stores/musicLibrary.ts`)
   builds all `MusicTrack` objects at once, after which `tracks` spread,
   `filtered`, and `pickDeck`→`groupTracks` all run O(n) on the main thread.

Rendering is **not** the problem — `VirtualList` is properly windowed (~30 DOM
nodes). The freeze is purely main-thread ingestion.

There is also **no index** backing `ORDER BY title COLLATE NOCASE`
(`media_store.rs`), so every list does a full scan+sort. Fine at 20k, quadratic
under paginated reads at 1M.

## Approach A — incremental, non-blocking ingestion (keep in-memory model)

The main thread (Rust's and the webview's JS) must never do O(n) work over the
whole library in one uninterruptible task. Page the data in, accumulate cheaply,
publish to the UI on a throttle, yield between pages.

### Backend (Rust)

1. `crates/hm-core/src/media_store.rs`
   - Add `CREATE INDEX IF NOT EXISTS idx_tracks_title_nocase ON tracks(title COLLATE NOCASE)`
     to schema init (one-time, backfills existing DBs).
   - `count_tracks() -> Result<i64>` — `SELECT COUNT(*) FROM tracks`.
   - `list_tracks_page(offset, limit) -> Result<Vec<LibraryTrack>>` — same
     ordering as `list_tracks` + `LIMIT ? OFFSET ?`.
2. `src-tauri/src/commands/library.rs`
   - `library_count` — `#[tauri::command(async)]`.
   - `library_list_page(offset, limit)` — `#[tauri::command(async)]`.
   - Make existing `library_list` `#[tauri::command(async)]` (kept for
     back-compat; removes its main-thread block).
3. `src-tauri/src/lib.rs` — register the two new commands.

### Frontend (JS)

4. `src/lib/ipc.ts` — `libraryCount(): Promise<number>`,
   `libraryListPage(offset, limit): Promise<LibraryTrack[]>`.
5. `src/stores/musicLibrary.ts`
   - Extract per-track map into `mapLocalTrack(t)`.
   - Replace `ensureLocal`'s single fetch with a paged loader,
     `loadLocalPaged({ fetchCount, fetchPage, publish, yield_, isStale })`,
     written as a **pure, dependency-injected** function (correct by inspection,
     no JS test runner in the project):
     - fetch count once (for a progress fraction);
     - loop `fetchPage(offset, PAGE=1000)`; push mapped tracks into a running
       `acc` (O(1) each); `await yield_()` between pages; stop on a short page;
     - **throttled publish** (~every 200ms + a final publish) so consumer
       derivations (`tracks` spread, `filtered`, `pickDeck`) fire a few times/sec
       instead of once per page;
     - check `isStale()` (the `gen` token) after every `await` — existing
       cancellation model; invalidate/reload still cancels cleanly.
   - Add `yieldToMain()` util (MessageChannel, `setTimeout(0)` fallback).
   - Store gains `localTotal` for the progress fraction.
6. Progress UX: loading/footer copy shows e.g. "Loading your music…
   12,000 / 234,000"; the climbing `library.count` pill already reflects
   accumulation.

Phone/cloud are left as-is — bounded (one device / one account), not the
reported problem.

### Why this satisfies "1M without freezing"

Each IPC page is a separate event-loop task, so the browser paints and handles
input between pages. Per-page JS work is tiny; the only O(n) bursts (consumer
derivations) are throttled to a few/sec and spaced by yields. The UI is
interactive from the first page and never blocks. Memory still grows with the
in-memory model — the one thing Approach B (SQL-windowed datasource) would
flatten — deferred by explicit choice.

## Testing

- Rust unit tests (`cargo test`, in-memory store): `count_tracks` and
  `list_tracks_page` — ordering, offset/limit bounds, last partial page, empty.
- `loadLocalPaged` is pure + injectable; verified by inspection and by
  `tsc --noEmit` (no JS test runner is configured in this project — noted
  honestly rather than adding one).
- `npm run build` (tsc + vite) and `cargo test` green.

## Out of scope

- Approach B (windowed SQL datasource for flat memory at 1M+).
- Phone/cloud ingestion changes.
- FTS5 search (current substring search retained).

---

## Addendum — disconnected drive leaves stale tracks (2026-06-26)

**Problem:** the `tracks` table persists rows by `path` forever; nothing
reconciles them against whether the file is reachable, so an unplugged external
drive's tracks keep rendering.

**Fix (two parts, both keep rows in the DB so a reconnect needs no re-scan):**
1. **Availability filtering at the query layer** (`media_store.rs`):
   `track_available(path, dir_cache)` checks the file's **parent directory**
   (`is_dir()`), cached per directory so a missing drive collapses to a handful
   of `stat`s. `list_tracks_page` now returns `LibraryPage { tracks, scanned }`
   — `tracks` is the reachable subset, `scanned` is raw DB rows read so the
   loader advances its offset correctly when rows are hidden. New
   `count_available_tracks()`. `count_tracks()`/`list_tracks` stay raw.
2. **Focus-triggered revalidation** (user-chosen over OS volume events):
   `App.tsx` listens for `window` `focus` → store `revalidateLocal()` probes
   `library_available_count` and, only if it differs from the shown count, flips
   `localLoad` to `idle` so the Library reloads (lazily if unmounted). One probe
   at a time (`revalidating` guard).

Frontend loader (`loadLocalPaged`) updated: `fetchPage` returns `LibraryPage`,
offset advances by `scanned`, `done = scanned < pageSize`, and publishes only
when the accumulator grew (so long stretches of hidden rows don't churn
re-renders). New IPC `libraryAvailableCount`; `LibraryPage` type added.

**Tests:** `page_hides_unreachable_files` (real temp dir/file vs a bogus path —
filtered out, `scanned` counts both, `count_available_tracks` = 1 of 2).
Granularity note: parent-dir existence (not per-file), which is the right unit
for unplugged-drive (mount point vanishes) and avoids a `stat` per file.
Per-focus `count_available_tracks` is a full path scan + cached dir stats —
fine at realistic sizes, heavier (off-main-thread) at 1M.
