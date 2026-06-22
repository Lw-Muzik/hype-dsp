# Open music files from the file manager — design

**Date:** 2026-06-22
**Status:** Approved, implementing

## Goal
Let the user open audio file(s) from the OS file manager (double-click, or
"Open With → HypeMuzik") and have them play in HypeMuzik. Opening **plays
immediately and replaces the queue** (multi-select → play the first, queue the
rest), and opened files are **also imported into the Library**.

## OS registration ("by default")
Add `bundle.fileAssociations` to `tauri.conf.json` for the app's supported
formats: `mp3, flac, wav, ogg, oga, m4a, aac, mp4, opus` (the existing
`AUDIO_EXTS` in `commands/library.rs`). This registers HypeMuzik as a handler:
it appears under **Open With** and *can be set as the default* app.

Honest limits:
- macOS/Windows do not let an app silently seize itself as THE system default;
  the OS/user makes that final choice. We register so it *can* be chosen and so
  "Open With → HypeMuzik" works immediately.
- Associations take effect only from an **installed bundle**, not `tauri dev`.
- Linux: the bundler writes the `.desktop` `MimeType`.

## Receiving the path(s)
Three OS behaviors, unified into one buffer + one frontend handler:
- **macOS:** files arrive as `RunEvent::Opened { urls }` (cold launch *and*
  while running). Switch the app from `.run(context)` to
  `.build(context)?.run(|app, event| …)` to catch it.
- **Windows/Linux:** the path arrives as a launch **argv**; a second launch
  while running is forwarded to the first instance via
  **`tauri-plugin-single-instance`** (callback delivers the new argv + focuses).

**Cold-launch race:** the webview may not be mounted when the path arrives, so
the backend buffers paths in a managed `PendingOpen(Mutex<Vec<String>>)`. The
frontend drains it on mount, and also subscribes to an `app:open_files` event
for warm opens.

## Data flow
```
OS open ─┬─ argv (Win/Linux startup)
         ├─ RunEvent::Opened (macOS, cold + warm)
         └─ single-instance cb (warm Win/Linux)
              → push paths to PendingOpen  +  (warm) emit "app:open_files"
Frontend providers.tsx on mount:
   takePendingOpenFiles()  ─┐
   onOpenFiles(handler)  ───┴─→ openFiles(paths)                    [backend]
        backend: filter to audio → index_paths() (import + read tags)
                 → return LibraryTrack[]
   → items = tracks.map(localItem) → playQueueItems(items, 0)   (replace queue)
   → useLibraryStore.refresh()   (bump version → Local list refreshes)
   → focus the main window
```
The existing `engine:now_playing` event still refines cover/tags after decode,
so the now-playing card fills in for free.

## Files
- `src-tauri/Cargo.toml` — add `tauri-plugin-single-instance`.
- `src-tauri/tauri.conf.json` — `bundle.fileAssociations`.
- `src-tauri/src/lib.rs` — register single-instance plugin **first**; manage
  `PendingOpen`; parse startup argv; switch to `.build().run(|app,event|…)` for
  `RunEvent::Opened`; register `open_files` + `take_pending_open`.
- `src-tauri/src/commands/open_with.rs` (new) — `PendingOpen` state +
  `open_files`/`take_pending_open`; reuses `index_paths`/`is_audio` from
  `library.rs` (marked `pub(crate)`).
- `src/lib/ipc.ts` — `openFiles`, `takePendingOpenFiles`, `onOpenFiles`.
- `src/app/providers.tsx` — wire the mount handler (reuses `localItem`,
  `playQueueItems`, `useLibraryStore.refresh`).

## Edge cases
- Non-audio / non-existent paths filtered out (no-op).
- macOS `file://` URLs converted to filesystem paths.
- Rapid repeat opens re-trigger replace-queue (last wins) — acceptable.
- Single-instance is a harmless no-op on macOS (the OS already routes `Opened`
  to the running app).

## Verification limits
`cargo check` + `tsc` confirm it compiles; a warm `app:open_files` emit can be
exercised. The actual Finder/Explorer double-click association can only be fully
verified from an **installed bundle**, which can't be produced/run in this
environment — flag as needs-packaged-build-to-fully-test.
