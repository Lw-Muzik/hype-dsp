# Stations: Radio + World TV — Design

**Date:** 2026-07-15
**Branch:** `feat/stations-tv`
**Status:** Approved design → implementation planning

## 1. Goal

Rename the **Radio** section to **Stations**, and host two kinds of live media under
it: **Radio** (the existing internet-radio feature, unchanged) and **TV** (new) — a
browsable catalog of publicly-available, free-to-air television channels from around
the world, played in a native, VLC-class video player that handles every format.

Radio streams as audio through the existing DSP engine. TV is video, so it plays in a
**dedicated native mpv window** with mpv's built-in on-screen controller.

## 2. The right resource (research outcome)

**[iptv-org](https://github.com/iptv-org/iptv)** is the canonical, open-source,
legally-clean catalog of publicly-available (free-to-air) TV channels — the exact TV
analog of radio-browser (which Radio already uses). ~5,800+ verified live streams,
170+ countries, with logos, categories, and languages, refreshed daily via CI.

We consume iptv-org's **pre-built M3U playlists** (not the raw 10 MB+ JSON), mirroring
the existing `radio::by_country` pattern:

| Purpose | Source | Size | Notes |
|---|---|---|---|
| Global catalog (search / "all TVs") | `…/iptv/index.m3u` | ~2.9 MB | One file; carries `tvg-logo`, `group-title` (category), `http-user-agent`, name, URL. Country derived from `tvg-id` suffix (`Name.cc@FEED`). Fetched once, disk-cached with TTL. |
| Browse by country | `…/iptv/countries/{cc}.m3u` | small | Self-contained; fetched on demand + cached. |
| Browse by category | `…/iptv/categories/{cat}.m3u` | small | Self-contained; fetched on demand + cached. |
| Offline / first-run | bundled `tv_seed.json` | tiny | Curated top world channels (mirrors `radio_seed.json`). |

Each `#EXTINF` line yields a self-contained channel record. `#EXTVLCOPT:http-user-agent`
/ inline `http-user-agent` and `http-referrer` are captured and passed to mpv verbatim
(many streams require them).

## 3. Playback engine (research outcome)

**mpv** (the media player), spawned as a **separate child process** that owns its own
native window and shows its **built-in OSC** (VLC-style: play/pause, seek bar for VOD,
volume, fullscreen, settings).

Why this over the alternatives:

- **Plays everything.** mpv is ffmpeg-backed: HLS (`.m3u8`), DASH, RTSP/RTMP, MPEG-TS,
  mp4, mkv — every container and codec that exists. This is the "like VLC, all formats"
  requirement.
- **CSP/CORS/proxy problem disappears.** mpv does its own native HTTP with per-stream
  `--user-agent`/`--referrer`; nothing touches the webview, so the app's locked-down CSP
  is irrelevant and no local proxy is needed.
- **Bulletproof, cross-platform, minimal code.** mpv owns windowing + OSC + reconnect on
  all three desktop OSes. Choosing a *dedicated* window (not an in-app overlay) sidesteps
  the known "WebKitGTK/Linux embedding is broken" and "app-wide transparency" problems
  entirely — we *want* mpv's own window.
- **Clean licensing.** Because mpv runs as a separate, unmodified process (aggregation,
  not linking — like launching VLC), its GPL is satisfied with no entanglement with the
  proprietary app. We ship the unmodified binary + its license text.

**Control:** mpv is launched once, idle, with `--input-ipc-server=<socket>` (Unix socket
on mac/Linux, named pipe on Windows). Channels are loaded/switched by writing JSON
commands to that socket (`loadfile`, property sets, `stop`); mpv events (title, `end-file`
reason, window closed) are read back and surfaced to the UI.

**Bundling (senior call):** ship the mpv binary + its dylibs inside the app bundle
(`resources/`), located at runtime, so end users install nothing — never `brew install
mpv`. Dev falls back to a `mpv` found on `PATH`.

**DSP note (accepted limitation):** TV audio plays through mpv, bypassing the app's
enhancement chain. When TV starts, the app's audio engine playback is paused so radio and
TV don't play at once.

## 4. Architecture

### 4.1 Navigation restructure

- `src/stores/ui.ts` — `Route` union: rename `"radio"` → `"stations"`.
- `src/app/routes.ts` — the `radio` route becomes `stations`: label **"Stations"**, icon
  `Tv` (lucide), tagline "Live radio and TV from around the world, streamed natively."
- `src/app/router.tsx` — `stations` → `StationsView`.
- **New** `src/features/stations/StationsView.tsx` — owns the `PageHeader` and a
  `Radio | TV` segmented toggle; renders `<RadioPanel/>` or `<TvPanel/>`.
- **Refactor** `src/features/radio/RadioView.tsx` → `src/features/stations/RadioPanel.tsx`
  — the existing radio toolbar + list, behaviour **identical**, minus its own header
  (StationsView owns it). No change to radio IPC, backend, or logic.

### 4.2 TV catalog (backend)

- **New** `crates/hm-media/src/tv.rs` — mirrors `radio.rs`:
  - `search(query) -> Vec<TvChannel>` — filters the cached global index; falls back to
    the query-filtered bundled seed on failure. Never errors.
  - `by_country(code) -> Vec<TvChannel>` — fetches + parses `countries/{cc}.m3u`, cached.
  - `by_category(id) -> Vec<TvChannel>` — fetches + parses `categories/{id}.m3u`, cached.
  - `categories() -> Vec<TvCategory>` — the fixed iptv-org category set.
  - Internal M3U parser: `#EXTINF` attrs (`tvg-id`, `tvg-logo`, `group-title`, inline
    `http-user-agent`/`http-referrer`), `#EXTVLCOPT:` lines, display name + quality, and
    the following URL line. Robust to malformed lines.
  - Disk cache under the app cache dir (`tv_index.json` + per-country/category files) with
    a TTL (default 7 days); `include_str!` bundled `tv_seed.json` for offline/first-run.
- **New** `crates/hm-core/src/types.rs::TvChannel`:
  ```rust
  pub struct TvChannel {
      pub id: String,
      pub name: String,
      pub url: String,
      pub logo: Option<String>,
      pub group: Option<String>,        // category (group-title)
      pub country: Option<String>,      // ISO 3166-1 alpha-2
      pub user_agent: Option<String>,
      pub referrer: Option<String>,
      pub quality: Option<String>,
  }
  ```
- **MediaStore** (`hm-core`) — add TV favorites alongside radio's, without touching radio:
  `list_tv_favorites`, `add_tv_favorite`, `remove_tv_favorite`, backed by a **new,
  separate `tv_favorites` table** (leaves the existing radio favorites schema untouched —
  lowest risk).
- **New** `src-tauri/src/commands/tv.rs` — Tauri commands wrapping the above:
  `tv_search`, `tv_by_country`, `tv_by_category`, `tv_categories`, `tv_countries`
  (full world list: code + name; frontend renders the flag), `tv_favorites_list`,
  `tv_favorite_add`, `tv_favorite_remove`. Same `(async)` discipline as `radio.rs`.

### 4.3 Video engine (native)

- **New crate** `crates/hm-video/` — owns the mpv process + IPC:
  - `VideoPlayer::ensure_running()` — spawn idle mpv:
    `mpv --idle=yes --force-window=yes --osc=yes --input-ipc-server=<sock>
     --force-media-title=… --title="HypeMuzik TV"` using the bundled binary (or PATH).
  - `play(channel)` — set `user-agent`/`referrer` props, `force-media-title`, then
    `loadfile <url>` over IPC.
  - `stop()` — `stop` command; `shutdown()` — quit + reap; window-closed and `end-file`
    events read from the socket and forwarded.
  - Cross-platform IPC transport (Unix socket vs Windows named pipe); binary locator
    (bundled resource path → PATH fallback).
- **New** `src-tauri/src/commands/video.rs` — `tv_play(channel)`, `tv_stop()`,
  `tv_player_status()`. `tv_play` pauses the `AudioEngine`. Emits `tv://ended`,
  `tv://closed` to the webview.
- **Packaging** — `src-tauri/tauri.conf.json` bundles the mpv binary + dylibs under
  `resources`/`externalBin` per-OS; `scripts/` gains a fetch-mpv step; `release.yml`
  matrix provides the per-OS binary. (macOS-first; Windows/Linux binaries wired in the
  same pass.)

### 4.4 TV UI (frontend)

- **New** `src/features/stations/TvPanel.tsx` — modes `browse` (search) | `country` |
  `category` | `favorites`:
  - `CountryGrid` — full **world** grid (all countries with channels), flag + name.
  - `CategoryGrid` — iptv-org categories (news, sports, movies, music, kids, …).
  - `ChannelList` — logo + name + (category · country · quality), click → `tvPlay`,
    star toggles favorite. Reuses the visual language of the radio `StationList`.
  - Graceful logo fallback (a `Tv` glyph), loading/empty states, offline seed — all
    mirroring the radio panel.
- **New IPC** in `src/lib/ipc.ts`: `tvSearch`, `tvByCountry`, `tvByCategory`,
  `tvCategories`, `tvCountries`, `tvFavoritesList/Add/Remove`, `tvPlay`, `tvStop`.
- **New types** in `src/lib/types.ts`: `TvChannel`, `TvCategory` (reuse `RadioCountry`
  shape for countries).

## 5. Data flow

**Browse country:** TvPanel → `tvByCountry(cc)` → `tv::by_country` → cached or fetch
`countries/{cc}.m3u` → parse → `Vec<TvChannel>` → grid.
**Play:** click channel → `tvPlay(channel)` → `video::play` → (ensure mpv) → set headers →
`loadfile url` → mpv window shows video + OSC. App audio engine paused.
**Favorite:** star → `tvFavoriteAdd/Remove` → MediaStore → list refresh.
**Search:** query → `tvSearch(q)` → filter cached global index (built from `index.m3u`,
lazily downloaded + cached) → results; falls back to seed when offline.

## 6. Error handling

- Catalog fetch failure → bundled seed (search) or empty state (country/category), with a
  friendly message — same contract as radio (`search`/`by_country` never error).
- mpv missing (dev without bundle, and not on PATH) → `tv_play` returns `IpcError`; UI
  toasts "TV engine unavailable."
- Dead/stalled stream → mpv's `end-file` reason surfaced as a toast ("Channel unavailable —
  try another"); mpv window stays idle for the next pick.
- Malformed M3U lines are skipped, not fatal.

## 7. Testing

- **Rust:** M3U parser unit tests (attrs, `#EXTVLCOPT`, quality, malformed input); catalog
  cache TTL/fallback tests; `MediaStore` tv-favorites round-trip; `hm-video` argv + JSON-IPC
  builders + socket-path/binary-locator logic (actual mpv spawn guarded by availability).
- **Frontend:** `tsc` clean, `vite build` clean.
- **Manual (dev, `brew install mpv`):** play a known-good HLS channel; verify window +
  OSC + channel switching + favorites persistence + offline seed + audio-engine pause.

## 8. Build order (phased)

1. **Stations restructure** — rename route, `StationsView` + toggle, extract `RadioPanel`.
   Radio parity fully preserved. (No new deps; low risk.)
2. **Video engine** — `hm-video` crate + `tv_play/tv_stop` commands + bundling; prove by
   playing a hardcoded HLS URL from a temporary button. (De-risks the hardest part first.)
3. **TV catalog** — `tv.rs` + `TvChannel` + MediaStore tv-favorites + `tv.rs` commands +
   IPC + seed.
4. **TV UI** — `TvPanel` (country/category/search/favorites, logos) wired to catalog +
   player.
5. **Polish** — states, error toasts, audio-engine pause, tests, packaging (`tauri.conf`
   resources + `release.yml` per-OS mpv), docs/memory.

## 9. Out of scope (YAGNI)

- Routing TV audio back through the DSP chain (OS loopback capture — large, fragile).
- In-app overlay/embedded video (app-wide transparency + compositing risk).
- EPG / program guides, recording, casting TV to phone, DRM/subscription channels.
- Changing anything about the existing Radio behaviour or backend.
