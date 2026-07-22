# YT Music one-click setup (yt-dlp + ffmpeg auto-install)

**Date:** 2026-07-22 · **Status:** approved

## Problem

Playing YT Music needs yt-dlp (and downloads want ffmpeg), but the app only
*detects* them — the Settings panel tells non-technical users to run
`winget install yt-dlp` in a terminal. That is a wall for exactly the users
the app targets. The fix: one in-app click, real progress, no terminals, no
external applications; the system handles download, verification, install and
staying current.

## Decisions (user-approved)

- **One click installs both** yt-dlp and ffmpeg (whichever are missing).
- **All platforms** (Windows is the pain point; mac/linux nearly free).
- **Silent background auto-update** of the app-managed yt-dlp (~weekly).
- Approach: in-app downloader of official standalone builds (approach A) —
  not winget/brew driving (interactive agreements, often blocked), not
  build-time bundling (stale within weeks; distributing GPL ffmpeg inside the
  app raises obligations a user-initiated download does not).

## Sources (verified 2026-07-22)

| Tool | Platform | Source | Integrity |
|---|---|---|---|
| yt-dlp | win x64 / win arm64 | `github.com/yt-dlp/yt-dlp` latest: `yt-dlp.exe` / `yt-dlp_arm64.exe` | release `SHA2-256SUMS` |
| yt-dlp | macOS (universal2) | `yt-dlp_macos` | same |
| yt-dlp | linux x64 / arm64 | `yt-dlp_linux` / `yt-dlp_linux_aarch64` | same |
| ffmpeg | win x64 / arm64 | `github.com/yt-dlp/FFmpeg-Builds` latest: `ffmpeg-master-latest-win{64,arm64}-gpl.zip` | release `checksums.sha256` |
| ffmpeg | linux x64 / arm64 | same repo, `…linux{64,arm64}-gpl.tar.xz` | same |
| ffmpeg | macOS arm64 / intel | `ffmpeg.martin-riedl.de/redirect/latest/macos/{arm64,amd64}/release/ffmpeg.zip` → versioned URL | `<versioned-url>.sha256` |

Checksum mismatch **fails closed** (same policy as the VB-CABLE installer).
yt-dlp is deliberately *latest*, never hash-pinned: it goes stale within weeks,
which is the very reason it was never bundled.

## Architecture

**`src-tauri/src/commands/ytmusic_setup.rs`** (new; pattern-sibling of
`cable.rs`, whose `powershell`/`expand_archive`/`path_arg` helpers become
`pub(crate)` and are reused):

- `ytmusic_setup` command `(async)`: for each missing tool — streaming
  download (reqwest, 64 KiB chunks) → SHA-256 verify → extract (ffmpeg only;
  hidden `Expand-Archive` on Windows, `ditto`/`tar` on mac/linux — every spawn
  `CREATE_NO_WINDOW`-safe) → locate `ffmpeg[.exe]`/`ffprobe[.exe]` in the tree
  (only those two are kept; the win64 archive is ~168 MB with extras we drop)
  → write to temp in `app_local_data_dir()/bin` → chmod 755 (unix) → atomic
  rename. Emits `ytmusic-setup-progress` `{tool, phase, received, total}`
  (throttled) for a real progress bar. Returns fresh `YtMusicStatus`.
- `spawn_auto_update(app)`: background thread at launch. Only when the
  *active* binary is the app-managed copy (a PATH/package-manager install is
  its manager's job) and the stamp file is >7 days old: run `yt-dlp -U`
  windowless (standalone builds self-update in place), rewrite stamp
  (write-then-rename). ffmpeg is never auto-updated — YouTube breakage never
  involves it.

**`crates/hm-ytmusic/src/ytdlp.rs`**: `set_managed_bin_dir(dir)` (OnceLock,
set from the Tauri setup hook — the crate stays Tauri-free). `find_binary()`
and `have_ffmpeg()` check the managed dir **before** PATH: the copy the app
keeps fresh is authoritative; self-managed users never populate it, so nothing
changes for them. ffmpeg beside yt-dlp is found by yt-dlp's own same-directory
lookup — no `--ffmpeg-location` plumbing.

**Frontend** (`YtMusicView.tsx` + new pure `setupProgress.ts` helper):
the missing-yt-dlp card becomes a primary **"Set up automatically"** button →
progress bar + human line ("Downloading yt-dlp… 12 of 17 MB") → success check;
inline error with Retry and the manual command as fallback. Terminal
instructions demoted to a "prefer to install it yourself?" disclosure. The
ffmpeg nag gains the same one-click button. Event via `listen()`; status
refreshed from the command's return value.

## Errors

Offline/HTTP failure, checksum mismatch, archive-missing-binary: each surfaces
an actionable message (with the manual-install path) through the existing
`IpcError` toastless inline-error slot of the card. Partial downloads never
touch the final path (temp + rename).

## Licensing note

ffmpeg builds are GPL: the app never distributes them — download happens on
explicit user action, to the user's machine, for the user's own use (same
posture as the VB-CABLE flow; noted in module docs).

## Testing

- Pure Rust: checksum-file parsing (both `sums` formats + single-token),
  per-OS asset selection, update-stamp due-logic, managed-dir precedence
  (via `find_binary_with`).
- Live (`--ignored`, repo convention): real latest-release checksum fetch +
  parse for the current platform's asset names.
- Frontend: `setupProgress.ts` unit tests (formatting, phase lines).
- `cargo xwin clippy --target x86_64-pc-windows-msvc` for the Windows arms.
