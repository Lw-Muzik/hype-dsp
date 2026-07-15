# YouTube Music — design

**Date:** 2026-07-15
**Status:** implemented on `feat/ytmusic`
**Scope:** browse and play YouTube Music playlists through the DSP chain; download tracks to the laptop or to a paired phone.

## Why this is not a normal integration

Three findings from research shaped every decision below.

**YT Music is a silo the official API can't see.** Data API v3 exposes playlists made in the YouTube *video* UI, not the YT Music library, liked songs, uploads, or album subscriptions. Google's own Data Portability schema models `youtube.music` as a resource group separate from `youtube.playlists`. There is no compliant way to read what the user means by "my YT Music playlists".

**The official API cannot return audio.** Not a quota limit — Developer Policy III.I.7 bans separating audio from video, which is what an audio player does by definition. III.I.14 bans reaching content through any non-API technology, which makes ytmusicapi, rustypipe, youtubei.js and yt-dlp all ToS violations by construction. There is no compliant version of this feature.

**The legal calculus shifted in Jan 2026.** *Cordova v. Huneault* (N.D. Cal.) held YouTube's rolling cipher to be an effective access control under DMCA §1201(a), and that fair use is not a defence to circumvention. It's a motion-to-dismiss denial, not binding precedent, and it was creator-v-creator rather than Google suing a tool author — but it is the first US decision rejecting the EFF theory the whole ecosystem's confidence rests on, and it targets exactly the signature-decipher step this feature needs.

Most YT Music desktop apps (YTMDesktop, Pear Desktop) dodge all of this by embedding a webview and letting Google's own player play the audio. **HypeMuzik cannot**: the product *is* the DSP chain, and a webview is a black-box audio path the chain can't reach.

### The position taken

Stream, never persist, in the core — Harmonoid's stance, the most defensible line available to a native-audio player. Downloads exist but are quarantined: opt-in, and dependent on a yt-dlp the user installs themselves.

## Architecture

Two halves, deliberately different shapes:

| | Metadata | Audio |
|---|---|---|
| Via | `ytmapi-rs` 0.3.2 (MIT) | `yt-dlp` on PATH |
| Sync? | async (tokio) | sync (spawns a process) |
| Needs yt-dlp? | **No** | Yes |
| Needs PO token? | No | Only non-Premium |

`ytmapi-rs` is MIT. **rustypipe was rejected despite being more capable: it's GPL-3.0, and linking it would make all of HypeMuzik GPL.**

### The seam that makes this cheap

`YtMusicState::stream_target(video_id) -> (String, Vec<(String, String)>)` deliberately mirrors `CloudState::stream_target`. That single signature match means YT Music reuses the entire existing streaming stack — `RadioStreamSource`, Range seeking, resume-on-drop, throughput EWMA, `StreamQueueSource` gapless/crossfade — with **no new playback machinery**.

Verified against the live CDN rather than assumed:

```
Content-Range: bytes 0-1023/3449447   ← honest 206, so seeking and resume work
```

Googlevideo URLs carry `expire=` and are pinned to the resolving IP. That's the same property that made Dropbox's temporary links need per-attempt resolution, and the queue resolver already re-resolves fresh on every attempt — so the existing code was already right for this.

### The decoder constraint (non-obvious, load-bearing)

YT Music's default `bestaudio` is **itag 251: Opus in WebM**. This workspace's symphonia build enables `aac`/`isomp4` but **neither an Opus decoder nor a WebM/Matroska demuxer**. An Opus URL would hand the engine bytes it reads as silence — a "why is there no sound" bug with no error anywhere.

So the format selector is pinned to `bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]` → itag 140, AAC-in-MP4, ~130 kbps. No quality loss (251 is 129 kbps). A track offering no m4a fails loudly as `NoCompatibleFormat`.

`AUDIO_FORMAT` has a unit test asserting it never accepts opus/webm, plus an `--ignored` live test asserting the real CDN still returns m4a.

### What is deliberately NOT here

- **No `player_client=web_music`.** It seems right for YT Music, but that client serves *no* audio formats without a PO token, while yt-dlp's default ladder returns itag 140 reliably. Choosing clients is the job of the tool we delegate to *because* it tracks YouTube's changes; overriding its defaults only breaks that. (This was caught by the live test — the fakes passed.)
- **No bundled yt-dlp.** YouTube changes fast enough that a pinned copy goes stale in weeks; bundling means owning an updater for a binary we don't control, plus notarization for a *changing* sidecar. (Prior art in this repo: the bundled mpv's fatal duplicate `LC_RPATH` killed the TV work.)
- **No tag preload / cover cache.** Cloud tracks list as bare filenames and need tags range-read out of the container — hence `CloudMetaCache` and the 4-worker preload. YT Music returns title/artist/album/duration up front. That machinery is not needed and was not copied.
- **No `CloudProvider::YouTubeMusic`.** YT Music has no folder tree, no file sizes, and cookie auth rather than OAuth. Folding it in would mean lying to `cloud.rs` in 14 match arms and fighting `fetch_stream_metadata`. It gets its own source instead. `cloud.rs`'s missing-trait problem is real but out of scope here.

## Auth

In-app webview → `music.youtube.com` login → harvest cookies. One login feeds both halves: `ytmapi-rs` gets a `Cookie:` header, yt-dlp gets a Netscape `cookies.txt`.

The payoff beyond private playlists: **yt-dlp uses the cookies to detect Premium, and Premium subscribers skip the GVS PO token requirement entirely** — removing the single most fragile dependency for a large slice of YT Music users.

Two subtleties:
- **Both `.youtube.com` and `.google.com` cookies are required.** The session lives on the former; `SAPISID`, which the API's auth hash derives from, is set on the latter. Capturing one yields a client that looks signed in and returns nothing.
- **Sign-in polls the cookie jar rather than matching a redirect URL.** The flow bounces through consent, 2FA and account-picker pages; any URL match would be a guess about Google's routing. The cookies are what we actually need.

### Storage

Google session cookies are **full account credentials** — whoever holds them is signed in as the user, everywhere. They go in the **OS keychain** (Keychain / Credential Manager / Secret Service), not the plaintext JSON that holds the cloud OAuth tokens.

yt-dlp only accepts cookies as a file, so they must touch disk for the length of a call. `CookieFile` bounds that: temp dir, `0600` set *before* the write, unlinked on drop.

## Downloads

Land in a configured folder (default `<Music>/HypeMuzik`), then get **indexed into the library as local tracks**. That's the point: once downloaded it seeks properly, plays offline, and survives yt-dlp breaking later. It stops being a YouTube Music thing and becomes a file the user owns.

yt-dlp picks the filename from track metadata, so the result is verified to be inside the download dir (`is_within`) rather than trusted.

### To a phone

Two phases — download to laptop, then upload the file — not a pipe from yt-dlp into the upload. Each phase is independently retryable, the file is complete before it's sent, and the laptop keeps a copy.

It also means the upload leg is just "send a local file", which is why `LinkState::upload` takes a path. **That fills a real gap: Phone Link could only ever pull *from* the phone.** `link_upload` exposes it for any local track.

`POST /upload` on the phone's shelf server is the **first desktop→phone write** in the subsystem. It needs no transport work: `hm-remote` is a byte-transparent QUIC tunnel where the desktop always dials, so it works for a phone on cellular exactly as on the LAN.

The linchpin on the phone is MediaStore registration — a file written to app-private storage is invisible to `OnAudioQuery().querySongs()`, so an uploaded track wouldn't appear in the phone's library or be streamable back.

### Security note

The bearer token that grants read access is the same one that authorises uploads — a read capability widened to a write one. Mitigation: the phone gates uploads behind its own opt-in setting, so pairing does not imply consent to write. Pre-existing weaknesses this inherits and does **not** fix: tokens never expire, have no scope, and `unpair` only forgets locally (a stale token stays valid on the phone forever).

## Failure modes

| Condition | Behaviour |
|---|---|
| yt-dlp missing | Playlists still browse fully. Only playback/download gated. Presented as a setup step with install instructions, not an error. |
| ffmpeg missing | Downloads work, without embedded tags/art. |
| Non-Premium, no PO token | `Blocked` — actionable ("sign in"), distinguished from a dead track. |
| Only Opus offered | `NoCompatibleFormat` — fails loudly rather than playing silence. |
| Track removed/region-locked | `Unavailable`. Listed but visibly unplayable. |
| Cookies expired | Pruned at load; state reads as signed out. |
| URL expired mid-queue | Resolver re-resolves per attempt. |
| One playlist fails | Skipped, not fatal — a broken playlist can't hide a library. |

`classify()` maps yt-dlp's stderr onto these, and is unit-tested, because the difference between "sign in" and "this track is gone" is the difference between a fixable prompt and a dead end.

## Testing

- `hm-ytmusic`: 33 unit tests. yt-dlp is behind a `YtDlpRunner` trait, so resolution/download are testable against canned output with no binary installed.
- One `--ignored` live test hits the real binary and CDN — the contract check to re-run after a yt-dlp or YouTube change. **It caught the `web_music` bug that every fake passed.**
- `hm-link`: `pct_encode` is tested against CRLF header injection; `ProgressReader` for byte-accuracy and pass-through.
