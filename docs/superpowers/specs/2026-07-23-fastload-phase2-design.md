# Fast YT Music Track Loading — Phase 2 Design

Date: 2026-07-23
Status: Approved (Phase 2 of the fast-load program; analysis §R4 + §R7 in
`.superpowers/sdd/fable-fastload-analysis.md`; Phase 1 shipped on main)

## Goal

Make **relaunch-and-play within the ~6h URL window** as fast as a warm
same-session play (~0.3s + stream open instead of a ~5.3s yt-dlp resolve), and
stop paying a fresh TLS handshake on every consecutive track. Two changes:

## R4 — Disk-persisted URL cache with probe-on-restore

The in-memory `resolved` cache (audio stream URLs, keyed by video id, pruned
by the CDN's own `expire=`) dies with the process while its entries live ~6h.

**Persistence model** (follows the crate-boundary precedent set by
`YtLibraryCache`: the hm-ytmusic crate owns the data, src-tauri owns paths and
file I/O):

- `ytdlp::StreamTarget` gains `Serialize`/`Deserialize` (internal file format,
  no camelCase needed).
- `YtMusicState` gains:
  - a `restored: TargetCache` field — disk-loaded entries quarantined apart
    from the live map, because a restart often means a network/IP change and
    googlevideo URLs are IP-bound: **a restored entry must be probed before
    first use**, while same-session entries keep their ~µs unprobed hits;
  - a monotonic generation counter bumped on every audio `remember()`, so the
    saver can skip writes when nothing changed;
  - `url_cache_snapshot(&self) -> Option<(u64, String)>` — (generation, JSON
    envelope of still-fresh audio entries: the union of the live map and the
    not-yet-probed `restored` map, live winning on conflict — otherwise a
    relaunch would shrink the file to only what got played); `None` when empty.
    The generation bumps on `remember()` and on a probe *dropping* a restored
    entry (the union changed); not on restore itself (that state came from the
    file). Promotion reuses `remember()` and so bumps too — the union is
    unchanged, making that one spare write per relaunch-play; accepted for the
    simpler code;
  - `restore_url_cache(&self, json: &str)` — parses the envelope, drops
    non-fresh entries, fills `restored`. Tolerant: version mismatch or
    garbage input is silently ignored (it is only a cache).
- Envelope: `{ "version": 1, "entries": { "<videoId>": StreamTarget, ... } }`.
- src-tauri (`src-tauri/src/ytmusic.rs` + setup in `lib.rs`):
  - file `ytmusic-stream-urls.json` in the same directory as the
    `YtLibraryCache` file, written with the same write-then-rename pattern;
  - restore once at startup right after `YtMusicState::load()`;
  - a periodic saver (60s interval, async task) that snapshots only when the
    generation moved, plus one final save on `RunEvent::ExitRequested`.

**Probe-on-first-use**: when `stream_target`/`prefetch` miss the live cache
but hit `restored`, issue one `GET` with `Range: bytes=0-1` and the entry's
own headers (~100–300ms). 200/206 → promote the entry into the live map (and
remove from `restored`); anything else (403, timeout ~5s, error) → drop it and
fall through to a normal resolve. Because Phase 1's `prefetch_batch` warms the
next tracks at queue start, restored entries usually get probed in the
background — the click itself rarely pays even the 300ms.

## R7 — Shared blocking HTTP clients

- `hm-audio` builds a fresh `reqwest::blocking::Client` per stream
  (`streaming.rs` ~line 581, `stream_queue.rs` ~lines 73-78) — no TLS/conn
  reuse across consecutive tracks on the same googlevideo host (~100–300ms per
  track start). Share ONE process-wide blocking client via `std::sync::OnceLock`
  (keep the existing 12s connect timeout).
- `hm-ytmusic`'s probe gets its own shared blocking client (new direct
  `reqwest` dep, `blocking` + `rustls-tls` features to match the workspace TLS
  choice). Known limitation, accepted: the probe's TLS warm does not transfer
  into hm-audio's pool (separate clients across crates); the R7 win is the
  per-track reuse inside hm-audio.

## Out of scope (later phases)

First-chunk RAM prefetch (deferred by the analysis until R1/R2/R5 prove
insufficient), incremental gapless start (Phase 3), native InnerTube resolve
(Phase 4). The video URL map stays memory-only (resolved on demand behind a
spinner; persistence buys little).

## Testing

- hm-ytmusic: snapshot/restore round-trip (fresh-only filter both directions,
  version-mismatch and garbage ignored, generation moves on remember and not
  on read); probe wire tests against a local TcpListener (206 → promoted into
  the live map; 403 → dropped from `restored`); the promote/drop plumbing
  factored so it is testable without spawning yt-dlp.
- hm-audio: existing suites stay green (the shared client is behaviorally
  identical); no new wire behavior.
- src-tauri: compile + clippy (thin I/O plumbing over tested crate APIs).
- Workspace gates as usual.
