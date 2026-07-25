# Free Movies via licensed distributor YouTube channels — implementation spec

**Extends** the existing Movies section (commit `8fb00eb`, Internet Archive). Adds modern (80s→now) full movies from **licensed distributor YouTube channels**, played through the **official YouTube IFrame embed**. Archive becomes a "Classics" grouping alongside.

## Legal / ToS boundary (non-negotiable)
- Source ONLY the verified rights-holder channels below. NEVER a piracy aggregator / "embed API".
- **Playback MUST use the official YouTube iframe embed — NOT the app's yt-dlp/googlevideo stream-extraction path** (that path is fine for music, but extracting *movie* streams breaches YouTube ToS). yt-dlp is used ONLY for keyless *discovery* (listing a channel's uploads + metadata), never to fetch the movie stream.
- The embed must: run YouTube's ads (never block/skip them), put no clickable overlay over the player video area, be ≥ 480×270, allow fullscreen, no background/audio-only, no download. (YouTube IFrame API Required Minimum Functionality.)

## Verified channels (curl/yt-dlp confirmed — use these exact ids)
| Shelf label | handle | channel_id |
|---|---|---|
| FilmRise | `@FilmRiseMovies` | `UC8IHAQMuiJdY6ALuhG7iU8Q` |
| Shout! Studios | `@ShoutStudios` | `UCpHaAKu74UHvcYCi2g_PvBQ` |
| Maverick Movies | `@MaverickMovies` | `UC2u3R3pjOiPZu4LtTlKkxdw` |
| Mosfilm (classics) | `@MosfilmRuOfficial` | `UCEK3tT7DcfWGWJpNEDBdWog` |

(WB's free-movie shelf is geo-flaky/unstable — SKIP for v1; the constant list must be trivially extendable to add more verified channels later.)

## Discovery (keyless, via the app's bundled yt-dlp)
`yt-dlp --flat-playlist -J "https://www.youtube.com/@<handle>/videos"` (or `.../channel/<id>/videos`) returns `{ entries: [{ id, title, duration (seconds), ... }] }`. **Filter to full movies:** `duration >= 3600` (60 min) — this drops the 1–2 min trailers/clips that share the channel. (Optionally also accept `duration >= 2400` when the title matches `/full movie/i`.) Thumbnail = `https://i.ytimg.com/vi/<id>/hqdefault.jpg`. Strip marketing suffixes like `| FULL MOVIE | ...` from the display title (keep it readable).

**REUSE the app's existing yt-dlp invocation** — it already spawns yt-dlp for YT Music (find the binary-resolution helper: managed-dir-wins-PATH, and the Windows `CREATE_NO_WINDOW` spawn flag). Do NOT hardcode a path. Run with a timeout; cache results in-memory per channel (and optionally to disk like the YT stream-url cache) — a channel listing is stable for hours.

## Rust (`src-tauri/src`)
Add to `commands/movies.rs` (or a sibling `movies_yt.rs` registered the same way):
- `yt_movies_channel(channel_id: String, limit: u32) -> Result<Vec<YtMovie>, IpcError>` — enumerate + filter as above. `YtMovie { video_id, title, duration_sec, thumbnail, channel_title }` (camelCase over IPC). Errors (yt-dlp missing/slow) → `IpcError`; the shelf degrades gracefully on the frontend.
- Pure helpers unit-tested from a fixture `yt-dlp -J` JSON: the full-movie duration filter + the title cleanup.
- Register the command in `lib.rs`.

## Frontend (`src`)
- **Movie model gains a `source` discriminator:** `type MovieSource = "archive" | "youtube"`. Archive cards keep `identifier`; YouTube cards carry `videoId`. A card's `source` drives which detail/player is used.
- `stores/movies.ts`: add the 4 YouTube distributor shelves (lazy-loaded, gen-counter-guarded like the archive shelves). Keep the existing archive shelves; group them in the UI under a **"Classics"** heading and the YouTube ones under their channel labels. Constant `YT_MOVIE_CHANNELS` (id/label) — commented as the single place to add channels.
- `MoviesView.tsx`: render the YouTube shelves + the archive "Classics" shelves. (Search stays archive-only for v1 — note that in a comment; YouTube channels aren't searchable keylessly without the Data API.)
- **New `YoutubeMoviePlayer.tsx`** — the official IFrame Player API:
  - Load `https://www.youtube.com/iframe_api` once (guard against double-load); create a player on `youtube-nocookie.com` embed for the `videoId`. Custom shell only OUTSIDE the video rect (title bar, a Close/back, a fullscreen toggle) — never an overlay over the player itself.
  - Handle the API `onError` (100/101/150 = removed/private/embedding-disabled, 2/5 = bad param/HTML5) → show a graceful "This title can't be embedded — Open on YouTube" fallback that opens `https://www.youtube.com/watch?v=<id>` via the app's existing external-open (`openExternalUrl` / `cloud.rs::open_url`).
  - Own the audio route like `MoviePlayer` does (stop the DSP engine on mount) so the movie's audio doesn't fight the player.
  - Fullscreen: allow the iframe to fullscreen (the YT player provides it) and/or a CSS `fixed inset-0` wrapper — match `MoviePlayer`'s technique.
- `MovieDetailView.tsx`: route to `MoviePlayer` (archive, direct MP4) or `YoutubeMoviePlayer` (youtube, iframe) by `source`. YouTube detail has no SD/HD toggle (YouTube handles quality) — hide it for that source.
- `lib/ipc.ts` + `lib/types.ts`: `ytMoviesChannel(channelId, limit)` wrapper + `YtMovie` type + the `source` field on the movie types.

## CSP (`src-tauri/tauri.conf.json`) — REQUIRED, scoped
The current CSP is `default-src 'self'; ...` with no `frame-src`/youtube `script-src`, which BLOCKS the embed. Add, scoped to YouTube only (do not widen to `*`):
- `frame-src https://www.youtube-nocookie.com https://www.youtube.com;`
- append to `script-src`: `'self' https://www.youtube.com https://s.ytimg.com` (the iframe_api loader + widget). If there's no explicit `script-src`, add one (don't rely on `default-src`).
- `img-src` already allows `https:` (covers `i.ytimg.com` thumbnails) — leave it.
Keep everything else unchanged. Add a comment on the new directives explaining they're for the official YouTube movie embed.

## Free section
Movies stays FREE — do NOT wrap any of this in `PremiumGate`.

## Verify before done
`cargo check -p hypemuzik` clean · `npx tsc --noEmit` clean · `npx vitest run` green (incl. new tests: the Rust full-movie filter + title-cleanup; the movies store's YouTube shelves; the source-based player routing). `npm run build` OK. Note: the iframe embed can't be exercised in unit tests / off-device — cover the discovery + routing logic in tests and leave live embed playback for on-device.
