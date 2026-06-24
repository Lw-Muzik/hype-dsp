# Online AutoEQ Database Fetch — Design

**Date:** 2026-06-24
**Status:** Approved (sourcing approach chosen by user: bundle index + fetch curve live)
**Scope:** Let the user search the [AutoEq](https://github.com/jaakkopasanen/AutoEq) headphone database by model and apply its correction curve, reusing the existing GraphicEQ import path. Closes the deferred follow-up to JamesDSP gap #2 (GraphicEQ import).

## 1. Goal
Type a headphone model, pick it from a list, and have its AutoEq correction curve applied to the 31-band EQ — without bundling thousands of curves or hand-pasting GraphicEQ text.

## 2. Sourcing decision (the fork — resolved)
The AutoEq DB is large (5,060+ `GraphicEQ.txt` files across many measurement sources). **Chosen:** bundle a generated **model→URL index** (a deduped snapshot, one entry per model, preferring canonical sources oratory1990 > crinacle > Rtings > Innerfidelity), and fetch only the **selected** model's `GraphicEQ.txt` live over the network.
- Index: `crates/hm-core/data/autoeq_index.json` — `[{name, source, url}]`, **3,938 deduped models**, ~927 KB, bundled via `include_str!`. Search is therefore instant + offline; only the chosen curve needs the network.
- Tradeoff: staleness — new headphones require regenerating the snapshot (a one-time tree scrape). Coverage is a large subset (GitHub's recursive tree API truncates), not 100% exhaustive — acceptable.

## 3. Architecture
- **`crates/hm-core/src/autoeq_db.rs`** — `AutoEqEntry { name, source, url }` (serde camelCase); `INDEX_JSON` via `include_str!`; lazily parsed once into a `OnceLock<Vec<AutoEqEntry>>`. `pub fn search(query, limit) -> Vec<AutoEqEntry>`: case-insensitive, every whitespace term must appear, ranked exact → name-prefix → word-prefix → contains, tie-break shorter-name then alphabetical, capped at `limit`; empty query → `[]`. Pure + fully unit-testable, offline.
- **Reuse:** the fetched curve string is fed into the existing `parse_graphic_eq → interpolate_to_iso_bands → recommended_preamp → engine.set_eq` pipeline. Extract a shared `apply_graphic_curve(&engine, &curve) -> Result<EqImportResult, IpcError>` helper in `commands/engine.rs`; both `engine_eq_import_graphic` and the new fetch command call it (DRY).
- **`src-tauri/src/commands/autoeq.rs`** — two commands:
  - `autoeq_search(query, limit?) -> Vec<AutoEqEntry>` (pure, delegates to `hm_core::autoeq_db::search`, default limit 50).
  - `autoeq_fetch_apply(engine, url) -> Result<EqImportResult, IpcError>` — `#[tauri::command(async)]` on a **sync** fn (Tauri runs it on a thread pool — no UI-thread blocking, matching `lyrics_fetch`/`identify_track`). SSRF guard: URL **must** start with `https://raw.githubusercontent.com/jaakkopasanen/AutoEq/`. `reqwest::blocking::Client` with a 15 s timeout + user-agent; non-2xx → `IpcError`; body → `apply_graphic_curve`.
- **Frontend** — `autoeqSearch`/`autoeqFetchApply` in `ipc.ts`; an `applyAutoEq(url)` store action mirroring `importGraphicEq` (sets `eq.bands`/`preGain`, enables EQ, clears `activePresetId`); a "Find headphone (AutoEQ)…" search affordance in `EqualizerView.tsx` beside "Import curve…": debounced (250 ms) search input → scrollable results list (name + source badge) → click applies + toasts.

## 4. Real-time safety / performance
No audio-thread involvement: the curve resolves to band values via the existing import path (which already only swaps EQ params; the limiter is the net). The network fetch runs on Tauri's command thread pool (never the UI thread, never the audio thread). Search is an in-memory scan of 3,938 short strings — sub-millisecond, off the audio thread. Disabled/unused = zero cost.

## 5. Security
The fetch command is the one new attack surface (an IPC command that pulls a URL). Mitigated by a strict host+path allowlist (`raw.githubusercontent.com/jaakkopasanen/AutoEq/`) — the renderer cannot make it fetch arbitrary hosts (no SSRF), and a request timeout bounds a hung fetch.

## 6. Testing
- **autoeq_db (core):** known models found (Sennheiser HD 600, Sony WH-1000XM4, Apple AirPods Pro, Moondrop Aria); empty query → `[]`; `limit` respected; ranking (exact/prefix beats mid-substring); multi-term (all terms must match); index parses (len > 3000).
- **URL guard:** `apply`-host validation accepts the AutoEq prefix, rejects other hosts.
- Network fetch + UI are integration-level (not unit-tested) — noted as not ear/on-device tested.
- Gates: `cargo test -p hm-core`, `cargo clippy --all-targets -- -D warnings`, `pnpm tsc --noEmit`.

## 7. Non-goals
Auto-refreshing/updating the bundled index at runtime; ParametricEQ/FixedBandEQ import; per-source curve selection in the UI (the index already picks the canonical source per model); offline fetch of the curve (only the index is offline).
