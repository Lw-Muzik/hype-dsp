//! One-click YT Music tooling setup — install yt-dlp (and ffmpeg) in-app.
//!
//! The Settings panel used to hand non-technical users a terminal command
//! (`winget install yt-dlp`); this module replaces that with:
//!
//!   streaming download (progress events) → SHA-256 verify → extract (ffmpeg
//!   only) → atomic install into `app_local_data_dir()/bin` → re-detect.
//!
//! No elevation, and **no console window can ever flash**: the download and
//! install are pure Rust; the only subprocesses are the hidden-window archive
//! extractors (`Expand-Archive` / `ditto` / `tar`).
//!
//! **Freshness over pinning.** Unlike the VB-CABLE flow, nothing here is
//! version-pinned: YouTube breaks old yt-dlp within weeks — the very reason it
//! was never bundled — so we always fetch the *latest* official standalone
//! build and verify it against the checksum file published in the same
//! release. A checksum mismatch still fails closed. The app-managed copy is
//! then kept current by [`spawn_auto_update`] (`yt-dlp -U`, windowless,
//! throttled to weekly); PATH/package-manager installs are never touched —
//! they belong to their package manager.
//!
//! **Licensing:** the ffmpeg builds are GPL. The app does not distribute
//! them — the download happens on explicit user action, to the user's own
//! machine, for the user's own use (same posture as the VB-CABLE flow).

use hm_core::IpcError;
use hm_ytmusic::ytdlp;
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};

/* ---- sources ---- */

/// The yt-dlp standalone asset for this OS/arch (official release names).
fn ytdlp_asset() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    return "yt-dlp_arm64.exe";
    #[cfg(all(target_os = "windows", not(target_arch = "aarch64")))]
    return "yt-dlp.exe";
    #[cfg(target_os = "macos")]
    return "yt-dlp_macos"; // universal2 — covers both Mac architectures
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "yt-dlp_linux_aarch64";
    #[cfg(all(target_os = "linux", not(target_arch = "aarch64")))]
    return "yt-dlp_linux";
}

/// The installed yt-dlp file name (differs from the asset name off-Windows).
fn ytdlp_bin_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

const YTDLP_LATEST: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";

/// Where this platform's ffmpeg comes from and how it is verified.
///
/// Windows/Linux use yt-dlp's own FFmpeg-Builds (same GitHub org, archives
/// with a `checksums.sha256` in the release). macOS uses martin-riedl.de,
/// whose `latest` redirect lands on a versioned URL with a `.sha256` beside
/// it — native builds for both Mac architectures, which the GitHub repo
/// doesn't provide.
enum FfmpegSource {
    /// GitHub release asset + the release's checksum file.
    // Constructed on Windows/Linux only; the match arms compile everywhere.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Github { asset: &'static str },
    /// Redirecting URL; checksum fetched from `<resolved-url>.sha256`.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Redirect { url: &'static str },
}

fn ffmpeg_source() -> FfmpegSource {
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    return FfmpegSource::Github {
        asset: "ffmpeg-master-latest-winarm64-gpl.zip",
    };
    #[cfg(all(target_os = "windows", not(target_arch = "aarch64")))]
    return FfmpegSource::Github {
        asset: "ffmpeg-master-latest-win64-gpl.zip",
    };
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return FfmpegSource::Redirect {
        url: "https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip",
    };
    #[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
    return FfmpegSource::Redirect {
        url: "https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/release/ffmpeg.zip",
    };
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return FfmpegSource::Github {
        asset: "ffmpeg-master-latest-linuxarm64-gpl.tar.xz",
    };
    #[cfg(all(target_os = "linux", not(target_arch = "aarch64")))]
    return FfmpegSource::Github {
        asset: "ffmpeg-master-latest-linux64-gpl.tar.xz",
    };
}

const FFMPEG_BUILDS_LATEST: &str =
    "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download";

/* ---- checksum files ---- */

/// The SHA-256 for `name` out of a checksum file.
///
/// Accepts both shapes in the wild: `sha256sum` listings (`<hex>  <name>`,
/// optionally `*<name>` for binary mode) and single-hash files (`<hex>`,
/// possibly with trailing metadata) as martin-riedl.de publishes.
fn checksum_for(sums: &str, name: &str) -> Option<String> {
    let mut single: Option<String> = None;
    for line in sums.lines() {
        let mut parts = line.split_whitespace();
        let Some(hex) = parts.next() else { continue };
        if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        match parts.next() {
            Some(file) if file.trim_start_matches('*') == name => {
                return Some(hex.to_ascii_lowercase());
            }
            Some(_) => {}
            // A bare hash names nothing: it only counts if the whole file
            // turns out to be a single-hash file (no named entry matched).
            None => {
                single.get_or_insert_with(|| hex.to_ascii_lowercase());
            }
        }
    }
    single
}

/* ---- download ---- */

/// What the UI hears while setup runs (event `ytmusic-setup-progress`).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupProgress {
    /// `yt-dlp` or `ffmpeg`.
    tool: &'static str,
    /// `downloading` / `verifying` / `installing`.
    phase: &'static str,
    received: u64,
    total: Option<u64>,
}

fn emit(app: &AppHandle, tool: &'static str, phase: &'static str, received: u64, total: Option<u64>) {
    // Narration is best-effort; setup must not fail over a UI event.
    let _ = app.emit(
        "ytmusic-setup-progress",
        SetupProgress {
            tool,
            phase,
            received,
            total,
        },
    );
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        // The largest download (win64 ffmpeg) is ~170 MB; leave slow
        // connections room without letting a stall hang forever.
        .timeout(std::time::Duration::from_secs(15 * 60))
        .build()
        .map_err(|e| format!("could not set up the download: {e}"))
}

/// Fetch a small text file (a checksum listing).
fn fetch_text(client: &reqwest::blocking::Client, url: &str) -> Result<String, String> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {} for {url}", resp.status()));
    }
    resp.text().map_err(|e| format!("download failed: {e}"))
}

/// Stream `url` into memory, reporting progress; returns the body and the
/// final URL after redirects (needed to find martin-riedl's `.sha256`).
fn download_with_progress(
    client: &reqwest::blocking::Client,
    url: &str,
    app: &AppHandle,
    tool: &'static str,
) -> Result<(Vec<u8>, String), String> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {} for {url}", resp.status()));
    }
    let final_url = resp.url().to_string();
    let total = resp.content_length();
    emit(app, tool, "downloading", 0, total);

    let mut reader = resp;
    let mut body: Vec<u8> = Vec::with_capacity(total.unwrap_or(0).min(256 * 1024 * 1024) as usize);
    let mut buf = [0u8; 64 * 1024];
    let mut last_emit = std::time::Instant::now();
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("download failed mid-stream: {e}"))?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
        // Throttled: progress is for humans, not for every 64 KiB chunk.
        if last_emit.elapsed() >= std::time::Duration::from_millis(150) {
            emit(app, tool, "downloading", body.len() as u64, total);
            last_emit = std::time::Instant::now();
        }
    }
    emit(app, tool, "downloading", body.len() as u64, total);
    Ok((body, final_url))
}

/* ---- install ---- */

/// The app-managed tools dir — must match what `set_managed_bin_dir` was
/// given at startup, or installs would land where detection never looks.
fn managed_dir() -> Result<PathBuf, String> {
    ytdlp::managed_bin_dir()
        .map(Path::to_path_buf)
        .ok_or_else(|| "the app-managed tools folder was never registered".to_string())
}

/// Write `bytes` as `name` into `dir`, atomically: temp file in the same dir,
/// executable bit (unix), then rename over the final path. A crash mid-write
/// can never leave a half-binary where detection finds it.
fn install_binary(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!(".{name}.part"));
    std::fs::write(&tmp, bytes).map_err(|e| format!("could not save {name}: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("could not mark {name} executable: {e}"))?;
    }
    let dest = dir.join(name);
    std::fs::rename(&tmp, &dest).map_err(|e| format!("could not install {name}: {e}"))?;
    Ok(dest)
}

/// Depth-first search for a file named `name` under `root` (the ffmpeg
/// archives nest their binaries under a versioned `…/bin/` folder whose name
/// we'd rather not hardcode).
fn find_in_tree(root: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_in_tree(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|f| f == name) {
            return Some(path);
        }
    }
    None
}

/// Extract `archive` into `dest` with a platform tool that never shows a
/// window. zip on Windows/macOS, tar.xz on Linux.
fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("could not create {}: {e}", dest.display()))?;
    #[cfg(target_os = "windows")]
    {
        super::cable::expand_archive(archive, dest)
    }
    #[cfg(target_os = "macos")]
    {
        // `ditto -xk` ships with macOS and handles every zip Finder can.
        let status = std::process::Command::new("ditto")
            .arg("-xk")
            .arg(archive)
            .arg(dest)
            .status()
            .map_err(|e| format!("could not extract the archive: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("could not extract the archive".into())
        }
    }
    #[cfg(target_os = "linux")]
    {
        // GNU tar is a given on desktop Linux; `-J` covers the .tar.xz.
        let status = std::process::Command::new("tar")
            .arg("-xJf")
            .arg(archive)
            .arg("-C")
            .arg(dest)
            .status()
            .map_err(|e| format!("could not extract the archive: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("could not extract the archive".into())
        }
    }
}

/// Downloaded bytes must match the published checksum or nothing is installed.
fn verify(app: &AppHandle, tool: &'static str, bytes: &[u8], expected: &str) -> Result<(), String> {
    emit(app, tool, "verifying", 0, None);
    let actual = super::cable::sha256_hex(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "the downloaded {tool} failed checksum verification — try again in a \
             few minutes (a release may be mid-publish), or install it manually"
        ))
    }
}

/// Install the latest official yt-dlp standalone build into the managed dir.
fn install_ytdlp(app: &AppHandle) -> Result<(), String> {
    let client = client()?;
    let asset = ytdlp_asset();
    let sums = fetch_text(&client, &format!("{YTDLP_LATEST}/SHA2-256SUMS"))?;
    let expected = checksum_for(&sums, asset)
        .ok_or_else(|| format!("the yt-dlp release checksums don't mention {asset}"))?;
    let (bytes, _) = download_with_progress(&client, &format!("{YTDLP_LATEST}/{asset}"), app, "yt-dlp")?;
    verify(app, "yt-dlp", &bytes, &expected)?;
    emit(app, "yt-dlp", "installing", 0, None);
    install_binary(&managed_dir()?, ytdlp_bin_name(), &bytes)?;
    Ok(())
}

/// Install ffmpeg (and ffprobe when the archive carries it) into the managed
/// dir. Only those binaries are kept — the win64 archive is ~170 MB of full
/// build, most of which the app has no use for.
fn install_ffmpeg(app: &AppHandle) -> Result<(), String> {
    let client = client()?;
    let (bytes, final_url, expected) = match ffmpeg_source() {
        FfmpegSource::Github { asset } => {
            let sums = fetch_text(&client, &format!("{FFMPEG_BUILDS_LATEST}/checksums.sha256"))?;
            let expected = checksum_for(&sums, asset)
                .ok_or_else(|| format!("the ffmpeg release checksums don't mention {asset}"))?;
            let (bytes, url) =
                download_with_progress(&client, &format!("{FFMPEG_BUILDS_LATEST}/{asset}"), app, "ffmpeg")?;
            (bytes, url, expected)
        }
        FfmpegSource::Redirect { url } => {
            let (bytes, final_url) = download_with_progress(&client, url, app, "ffmpeg")?;
            // The checksum lives beside the *versioned* file the redirect
            // resolved to — fetching it via the alias would 404.
            let sums = fetch_text(&client, &format!("{final_url}.sha256"))?;
            let name = final_url.rsplit('/').next().unwrap_or_default().to_string();
            let expected = checksum_for(&sums, &name)
                .ok_or_else(|| "the ffmpeg download published no readable checksum".to_string())?;
            (bytes, final_url, expected)
        }
    };
    verify(app, "ffmpeg", &bytes, &expected)?;

    emit(app, "ffmpeg", "installing", 0, None);
    let dir = managed_dir()?;
    let scratch = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("could not resolve the app cache folder: {e}"))?
        .join("ffmpeg-setup");
    // A previous half-finished attempt must not satisfy today's find.
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)
        .map_err(|e| format!("could not create the extraction folder: {e}"))?;
    let archive_name = final_url.rsplit('/').next().unwrap_or("ffmpeg-archive");
    let archive = scratch.join(archive_name);
    std::fs::write(&archive, &bytes).map_err(|e| format!("could not save the archive: {e}"))?;
    extract_archive(&archive, &scratch)?;

    let (ffmpeg_name, ffprobe_name) = if cfg!(windows) {
        ("ffmpeg.exe", "ffprobe.exe")
    } else {
        ("ffmpeg", "ffprobe")
    };
    let ffmpeg = find_in_tree(&scratch, ffmpeg_name)
        .ok_or_else(|| "the ffmpeg archive did not contain an ffmpeg binary".to_string())?;
    let bytes = std::fs::read(&ffmpeg).map_err(|e| format!("could not read the extracted ffmpeg: {e}"))?;
    install_binary(&dir, ffmpeg_name, &bytes)?;
    // ffprobe rides along when present (yt-dlp uses it for some postprocessing);
    // its absence (martin-riedl ships it separately) is not a failure.
    if let Some(ffprobe) = find_in_tree(&scratch, ffprobe_name) {
        if let Ok(bytes) = std::fs::read(&ffprobe) {
            let _ = install_binary(&dir, ffprobe_name, &bytes);
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}

/* ---- the command ---- */

/// Install whatever YT Music tooling is missing (yt-dlp, then ffmpeg), with
/// progress events, and return the resulting status. Present tools are left
/// exactly as found — a PATH install is never shadowed by a needless copy.
// `(async)`: long downloads — never main-thread work.
#[tauri::command(async)]
pub fn ytmusic_setup(
    app: AppHandle,
    state: tauri::State<'_, hm_ytmusic::YtMusicState>,
) -> Result<hm_ytmusic::YtMusicStatus, IpcError> {
    let wrap = |e: String| IpcError::new("ytmusic-setup", e);
    if ytdlp::find_binary().is_none() {
        install_ytdlp(&app).map_err(wrap)?;
        // A fresh install is current by definition — start the update clock.
        stamp_update_check();
    }
    if !ytdlp::have_ffmpeg() {
        install_ffmpeg(&app).map_err(wrap)?;
    }
    Ok(state.status())
}

/* ---- keeping the managed copy current ---- */

/// How long an update stamp holds before the next `-U` check.
const UPDATE_EVERY: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// Whether a stored stamp (unix seconds, as text) is old enough to act on.
/// Unreadable/garbage stamps count as due — failing open keeps yt-dlp fresh.
fn update_due(stamp: Option<&str>, now_secs: u64) -> bool {
    match stamp.and_then(|s| s.trim().parse::<u64>().ok()) {
        Some(then) => now_secs.saturating_sub(then) >= UPDATE_EVERY.as_secs(),
        None => true,
    }
}

fn stamp_path() -> Option<PathBuf> {
    Some(ytdlp::managed_bin_dir()?.join(".yt-dlp-update-stamp"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn stamp_update_check() {
    if let Some(path) = stamp_path() {
        // Write-then-rename, like every other state file in this app.
        let tmp = path.with_extension("part");
        if std::fs::write(&tmp, now_secs().to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Weekly, in the background: self-update the *app-managed* yt-dlp.
///
/// Only the managed copy — it is the official standalone build (which is what
/// `-U` knows how to replace) and nothing else owns it. A PATH copy belongs
/// to winget/brew/pipx; touching it would fight the user's package manager.
/// ffmpeg is deliberately never updated: YouTube breakage never involves it.
pub fn spawn_auto_update() {
    std::thread::spawn(move || {
        let Some(managed) = ytdlp::managed_bin_dir() else {
            return;
        };
        let bin = managed.join(ytdlp_bin_name());
        // Managed copy must exist *and* be the active one (a PATH copy that
        // shadows it would make updating ours pointless).
        if !bin.is_file() || ytdlp::find_binary().as_deref() != Some(bin.as_path()) {
            return;
        }
        let stamp = stamp_path().and_then(|p| std::fs::read_to_string(p).ok());
        if !update_due(stamp.as_deref(), now_secs()) {
            return;
        }
        // Stamp before running: a broken updater must not retry every launch.
        stamp_update_check();
        #[allow(unused_mut)]
        let mut cmd = std::process::Command::new(&bin);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        match cmd.arg("-U").output() {
            Ok(out) if out.status.success() => {}
            Ok(out) => eprintln!(
                "[ytmusic-setup] yt-dlp -U exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(e) => eprintln!("[ytmusic-setup] could not run yt-dlp -U: {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sha256sum`-style listing: the named entry, not the first line, wins.
    #[test]
    fn checksum_for_named_listing() {
        let sums = "aaaa000000000000000000000000000000000000000000000000000000000000  other.zip\n\
                    bbbb000000000000000000000000000000000000000000000000000000000000  target.zip\n";
        assert_eq!(
            checksum_for(sums, "target.zip").as_deref(),
            Some("bbbb000000000000000000000000000000000000000000000000000000000000")
        );
    }

    /// Binary-mode marker (`*name`) must still match.
    #[test]
    fn checksum_for_binary_marker() {
        let sums =
            "cccc000000000000000000000000000000000000000000000000000000000000 *target.zip\n";
        assert_eq!(
            checksum_for(sums, "target.zip").as_deref(),
            Some("cccc000000000000000000000000000000000000000000000000000000000000")
        );
    }

    /// A single bare hash (martin-riedl style) applies to whatever was asked.
    #[test]
    fn checksum_for_single_hash_file() {
        let sums = "DDDD000000000000000000000000000000000000000000000000000000000000\n";
        assert_eq!(
            checksum_for(sums, "ffmpeg.zip").as_deref(),
            Some("dddd000000000000000000000000000000000000000000000000000000000000")
        );
    }

    /// No matching name and no bare hash → no checksum, which fails closed.
    #[test]
    fn checksum_for_missing_entry() {
        let sums = "eeee000000000000000000000000000000000000000000000000000000000000  other.zip\n";
        assert_eq!(checksum_for(sums, "target.zip"), None);
    }

    /// Garbage lines (short hex, prose) must never be taken for a checksum.
    #[test]
    fn checksum_for_ignores_garbage() {
        let sums = "not a checksum at all\nabcdef  short.zip\n";
        assert_eq!(checksum_for(sums, "short.zip"), None);
    }

    /// The per-OS asset names must be ones the releases actually publish.
    #[test]
    fn asset_names_are_release_shaped() {
        let a = ytdlp_asset();
        assert!(a.starts_with("yt-dlp"), "unexpected asset: {a}");
        match ffmpeg_source() {
            FfmpegSource::Github { asset } => {
                assert!(asset.starts_with("ffmpeg-master-latest-"), "unexpected: {asset}");
            }
            FfmpegSource::Redirect { url } => {
                assert!(url.starts_with("https://ffmpeg.martin-riedl.de/"), "unexpected: {url}");
            }
        }
    }

    #[test]
    fn update_due_logic() {
        let now = 10_000_000;
        // Fresh stamp → not due.
        assert!(!update_due(Some(&now.to_string()), now));
        // A week and a bit ago → due.
        assert!(update_due(Some(&(now - 8 * 24 * 60 * 60).to_string()), now));
        // Missing or unreadable stamps fail open (due).
        assert!(update_due(None, now));
        assert!(update_due(Some("not-a-number"), now));
    }

    /// Live check (network): the current release really publishes the assets
    /// and checksums this module asks for — run with `--ignored`.
    #[test]
    #[ignore]
    fn live_release_publishes_our_assets() {
        let client = client().unwrap();
        let sums = fetch_text(&client, &format!("{YTDLP_LATEST}/SHA2-256SUMS")).unwrap();
        assert!(
            checksum_for(&sums, ytdlp_asset()).is_some(),
            "yt-dlp release no longer publishes {}",
            ytdlp_asset()
        );
        if let FfmpegSource::Github { asset } = ffmpeg_source() {
            let sums =
                fetch_text(&client, &format!("{FFMPEG_BUILDS_LATEST}/checksums.sha256")).unwrap();
            assert!(
                checksum_for(&sums, asset).is_some(),
                "FFmpeg-Builds release no longer publishes {asset}"
            );
        }
    }
}
