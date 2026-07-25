# Movies section (Internet Archive) — implementation spec

**Repo:** `hypemuzik-desktop` (Tauri 2, Rust `src-tauri` + React/TS `src`).
**Goal:** a legal "Movies" section that browses + streams public-domain / Creative-Commons films from the **Internet Archive** (archive.org) — its public API, no key. NOT vidbox or any pirate source.

## Verified API (grounded against the live endpoints — do not change the shapes)

### Search
`GET https://archive.org/advancedsearch.php` with query params:
- `q` = `collection:(<collection>) AND mediatype:(movies)` (browse) or, for search, `(<terms>) AND mediatype:(movies) AND collection:(feature_films OR SciFi_Horror OR silent_films OR classic_cartoons OR classic_tv OR prelinger OR short_films)`
- repeated `fl[]=identifier`, `fl[]=title`, `fl[]=year`, `fl[]=downloads`
- `sort[]=downloads desc` (popularity)
- `rows=<n>` (e.g. 30), `page=<p>` (1-based), `output=json`

Response JSON: `response.numFound` (int), `response.docs[]` = `{ identifier: string, title: string, year?: number, downloads?: number }`. (`year` and `downloads` may be absent on some docs — tolerate.)

### Resolve (metadata → playable file)
`GET https://archive.org/metadata/<identifier>` → `{ metadata: {title, year?, date?, description?, runtime?}, files: [{name, format, size, ...}] }`.
Video formats seen: `h.264` (HD .mp4), `512Kb MPEG4` (SD .mp4, fast-start), `Ogg Video` (.ogv). Build streams:
- **SD** (default): the `512Kb MPEG4` file; fall back to `h.264` if absent.
- **HD**: the `h.264` file (if present).
- URL for a file: `https://archive.org/download/<identifier>/<name>` (302 → CDN node, **Range-supported**, plays directly in a `<video>`).
`description` may contain HTML — strip tags for display.

### Poster
`https://archive.org/services/img/<identifier>` → an image (200). Use as the poster/thumbnail `src` directly.

## Confirmed collections (all return films — use exactly these ids/casing)
| Shelf label | collection id | count |
|---|---|---|
| Feature Films | `feature_films` | 28406 |
| Sci-Fi & Horror | `SciFi_Horror` | 954 |
| Silent Films | `silent_films` | 3526 |
| Classic Cartoons | `classic_cartoons` | 81 |
| Classic TV | `classic_tv` | 11158 |
| Prelinger Archives | `prelinger` | 10369 |
| Short Films | `short_films` | 3139 |

(`film_noir`, `comedy_films`, `documentaryfilms` return 0 — do NOT use. You may add more only after curl-verifying `numFound > 0`.)

## Rust (`src-tauri/src/commands/movies.rs`, new)
Follow the existing command-module conventions (see `commands/ytmusic.rs` / `commands/stations`/`tv` or `commands/account.rs` for the reqwest::blocking + `#[tauri::command(async)]` + `IpcError` pattern, and a shared reqwest client with a timeout + a `User-Agent`).
- `movies_browse(collection: String, page: u32) -> Result<Vec<MovieCard>, IpcError>` — one collection's popular films. `MovieCard { identifier, title, year: Option<u32>, poster_url, downloads: Option<u64> }` (camelCase over IPC).
- `movies_search(query: String, page: u32) -> Result<Vec<MovieCard>, IpcError>` — across the whitelisted collections above.
- `movies_resolve(identifier: String) -> Result<MovieDetail, IpcError>` — `MovieDetail { identifier, title, year, description, poster_url, streams: Vec<MovieStream> }`, `MovieStream { label: "SD"|"HD", url, mime: "video/mp4" }` (SD first).
- Register all three in `src-tauri/src/lib.rs` invoke_handler.
- Pure helpers unit-tested from fixture JSON: parsing `response.docs` → `MovieCard`; picking SD/HD from a `files` list; HTML-tag strip. Put a couple of `#[test]`s in the module.

## Frontend (`src`)
- `src/lib/ipc.ts`: `moviesBrowse(collection, page)`, `moviesSearch(query, page)`, `moviesResolve(identifier)` wrappers + the `Movie`/`MovieDetail`/`MovieStream` types in `src/lib/types.ts`.
- `src/stores/movies.ts`: browse-by-collection cache, search results + query, the opened movie + resolved detail, loading flags.
- **Sidebar + router:** add a "Movies" entry (a film/clapperboard lucide icon) to `src/components/Sidebar.tsx` and a route in `src/app/router.tsx` (match how existing views like Stations/Explore are registered).
- `src/features/movies/MoviesView.tsx`:
  - A search input (debounced) → `moviesSearch`.
  - When not searching: **shelves** — one horizontal row of poster cards per confirmed collection (lazy-load each shelf's first page). Reuse the Explore/Stations shelf + card patterns and `VirtualList`/virtualization already in the repo for long rows/grids.
  - When searching: a virtualized poster **grid** of results.
  - Poster card: `services/img` poster, title, year; click → open detail.
- Movie **detail + player** (`MovieDetailView` or a modal): poster/title/year/synopsis + a **Play** button; **SD/HD toggle** (default SD). Play via the app's EXISTING video mechanism — find how Stations/TV or the YT music-video stage renders `<video>` (HLS/direct) and reuse it; an Archive stream is a plain `<video src=download-url>` with native Range, fullscreen-capable. Only add a Rust range-proxy fallback if a direct `<video src>` demonstrably fails (it shouldn't — 302 + Range verified).
- Do NOT disturb music playback: opening a movie should pause/own the audio route sensibly like the existing video stage does (match that behaviour).
- Tests (vitest): movies store (browse/search/resolve state) + MoviesView renders shelves and a grid on search; the SD/HD stream selection.

## Scope
v1 = browse shelves + search + poster grid + detail + in-app playback (SD default, HD toggle) + fullscreen. **Deferred:** subtitles (Archive has SubRip files), continue-watching/resume, downloads.

## Verify before done
`cargo check -p hypemuzik` clean · `npx tsc --noEmit` clean · `npx vitest run` green (incl. new tests). Some Linux-only audio files won't compile on macOS — pre-existing/expected; the new movies code MUST compile & type-check.
