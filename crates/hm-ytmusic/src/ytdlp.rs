//! yt-dlp integration: stream-URL resolution and downloads.
//!
//! On the playback path yt-dlp is a *resolver*, not a downloader: `--print`
//! hands back the CDN URL plus the headers it must be fetched with, without
//! transferring a byte. That is exactly the `(url, headers)` shape the audio
//! engine's stream sources already consume, so YT Music reaches playback through
//! the same code as Dropbox. Downloading is a separate, opt-in call.
//!
//! The binary is deliberately **not bundled**: YouTube changes often enough that
//! a pinned copy goes stale within weeks, and shipping one would mean owning an
//! updater for a binary we don't control. We look it up on PATH instead and let
//! the user's own install stay current.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Format selector: strictly AAC-in-MP4 (itag 140, ~128 kbps).
///
/// YT Music's own `bestaudio` is itag 251 — Opus in WebM — which this app cannot
/// decode: the workspace symphonia build enables `aac`/`isomp4` but neither an
/// Opus decoder nor a WebM/Matroska demuxer. An Opus URL would hand the engine
/// bytes it reads as silence rather than an error, so we ask for m4a and nothing
/// else. A clear failure beats a track that spins with no sound.
const AUDIO_FORMAT: &str = "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]";

/// Video-only rendition for the picture beside DSP-chain audio.
///
/// Two pins, both load-bearing:
///
/// * **`vcodec^=avc1`.** Plain `bestvideo[ext=mp4]` prefers `av01` — YouTube
///   offers it at every rung — and WKWebView decodes AV1 on almost no Mac. It
///   would show a blank frame and report nothing, which is the Opus trap in
///   [`AUDIO_FORMAT`] wearing a different hat: the "best" format is the one the
///   player can't play, and it fails silently.
/// * **`height<=720`.** 1080p is ~56 MiB for a 3-minute video against ~15 MiB at
///   720p, and this streams *alongside* the audio, not instead of it.
///
/// Video-only on purpose: the element is muted and only ever a picture, so a
/// rendition with no audio track makes bypassing the enhancement chain
/// impossible rather than merely discouraged.
const VIDEO_FORMAT: &str =
    "bestvideo[ext=mp4][vcodec^=avc1][height<=720]/bestvideo[ext=mp4][vcodec^=avc1]";

/// Fields we ask `--print` for, in order. Each yields one stdout line.
const PRINT_FIELDS: &[&str] = &[
    "%(url)s",
    "%(ext)s",
    "%(format_id)s",
    "%(abr)s",
    "%(http_headers)j",
];

/// Placeholder yt-dlp prints for a field it has no value for.
const NA: &str = "NA";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YtDlpError {
    /// No yt-dlp on PATH. The UI turns this into install instructions rather
    /// than an error toast — it's a setup step, not a fault.
    NotInstalled,
    /// yt-dlp ran but YouTube won't serve this track (private, removed,
    /// region-locked, age-gated).
    Unavailable(String),
    /// YouTube rejected the request itself — typically a missing PO token on a
    /// non-Premium account, or expired cookies. Actionable by re-signing-in.
    Blocked(String),
    /// The m4a selector ([`AUDIO_FORMAT`]) matched nothing.
    ///
    /// Carries what yt-dlp actually said. It used to carry nothing, and its
    /// message asserted "only Opus" — a cause the code never observes. That
    /// claim survived three separate investigations of tracks which, checked by
    /// hand, offered itag 140 the whole time. An error that names a cause it
    /// cannot see costs more than one that admits ignorance.
    NoCompatibleFormat(String),
    /// YouTube served **no formats at all**.
    ///
    /// Kept apart from [`Self::NoCompatibleFormat`] because it reads like a codec
    /// problem and almost never is: it means the extraction itself came back
    /// empty (a client YouTube refused, a bot check that didn't name itself, a
    /// PO-token requirement). Reporting it as "only Opus" sent a real
    /// investigation down the wrong path for hours, so this one carries yt-dlp's
    /// own words instead of a guess.
    NoFormats(String),
    Failed(String),
}

impl std::fmt::Display for YtDlpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled => write!(f, "yt-dlp is not installed"),
            Self::Unavailable(m) => write!(f, "track unavailable: {m}"),
            Self::Blocked(m) => write!(f, "YouTube blocked the request: {m}"),
            Self::NoCompatibleFormat(m) => write!(f, "couldn't get a playable audio format — {m}"),
            Self::NoFormats(m) => write!(
                f,
                "YouTube returned no playable formats for this track — {m}"
            ),
            Self::Failed(m) => write!(f, "yt-dlp failed: {m}"),
        }
    }
}

impl std::error::Error for YtDlpError {}

/// A resolved, directly-fetchable audio stream.
///
/// `url` is IP-bound and dated — googlevideo stamps `ip=` and `expire=` into it.
/// Cacheable, but only against both: [`StreamTarget::expires_at`] carries the
/// stated deadline, while the address binding has no such marker, so a url can
/// die early if the network changes underneath it. A caller that holds these
/// needs a way to drop one that fails before its time, or a transient failure
/// becomes a permanent one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamTarget {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub ext: String,
    pub format_id: String,
    pub abr_kbps: Option<u32>,
    /// Unix seconds after which the CDN stops honouring `url`, read from its own
    /// `expire=` parameter. `None` when the url doesn't carry one, which reads as
    /// "assume nothing" — a caching caller must treat it as immediately stale
    /// rather than guess a lifetime.
    ///
    /// googlevideo issues these ~6 hours out, so the url long outlives the track.
    pub expires_at: Option<u64>,
}

/// Reads `expire=` out of a resolved url.
///
/// Hand-scanned rather than parsed with a url crate: this is one query parameter
/// on a url we were just handed, and the whole point is to learn the deadline the
/// CDN already told us, not to model urls. An unreadable or absent value is
/// `None` — never a default — because a wrong deadline is worse than no deadline:
/// too long serves a dead url, too short throws away a good one.
fn parse_expiry(url: &str) -> Option<u64> {
    let query = url.split_once('?')?.1;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("expire="))
        .and_then(|v| v.parse().ok())
}

/// Runs yt-dlp. Behind a trait so resolution and downloads are testable against
/// canned output with no binary installed.
pub trait YtDlpRunner: Send + Sync {
    /// Runs yt-dlp with `args`, returning stdout on success.
    fn run(&self, args: &[String]) -> Result<String, YtDlpError>;

    /// Runs yt-dlp, handing each stdout line to `on_line` as it arrives, and
    /// returns the full stdout. Downloads use this for progress; the default
    /// replays `run`'s output so fakes only need to implement `run`.
    fn run_streaming(
        &self,
        args: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> Result<String, YtDlpError> {
        let out = self.run(args)?;
        for line in out.lines() {
            on_line(line);
        }
        Ok(out)
    }
}

/// Where yt-dlp commonly lands beyond PATH. A GUI app launched from Finder or a
/// desktop entry inherits a minimal PATH that usually omits all of these, so a
/// PATH miss alone doesn't mean "not installed".
#[cfg(target_os = "macos")]
const EXTRA_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/local/bin",
    "/usr/bin",
];
#[cfg(target_os = "linux")]
const EXTRA_DIRS: &[&str] = &["/usr/bin", "/usr/local/bin", "/snap/bin", "/var/lib/flatpak/exports/bin"];
#[cfg(target_os = "windows")]
const EXTRA_DIRS: &[&str] = &[];

#[cfg(target_os = "windows")]
const BIN_NAME: &str = "yt-dlp.exe";
#[cfg(not(target_os = "windows"))]
const BIN_NAME: &str = "yt-dlp";

/// The app-managed install dir (the one-click setup's target), set once at
/// startup by the host app. Lives here rather than being passed through every
/// call so the crate stays host-agnostic: only the host knows its data dir.
static MANAGED_BIN_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Registers where the host app installs tools it manages itself. Later calls
/// are ignored (the dir is fixed for the process lifetime).
pub fn set_managed_bin_dir(dir: PathBuf) {
    let _ = MANAGED_BIN_DIR.set(dir);
}

/// The registered app-managed install dir, if the host set one.
pub fn managed_bin_dir() -> Option<&'static Path> {
    MANAGED_BIN_DIR.get().map(PathBuf::as_path)
}

/// Locates yt-dlp: the app-managed dir first, then PATH, then the usual
/// install dirs, then `~/.local/bin`.
///
/// Managed-first is deliberate: that copy is the one the app installed and
/// keeps auto-updated, so it must win over whatever (possibly stale) copy is
/// on PATH. Users who manage their own install never populate the managed
/// dir, so for them nothing changes.
pub fn find_binary() -> Option<PathBuf> {
    find_binary_with(managed_bin_dir())
}

/// [`find_binary`] with the managed dir passed explicitly (testable).
fn find_binary_with(managed: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = managed {
        let candidate = dir.join(BIN_NAME);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(BIN_NAME);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    for dir in EXTRA_DIRS {
        let candidate = Path::new(dir).join(BIN_NAME);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = Path::new(&home).join(".local/bin").join(BIN_NAME);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Runs the real binary.
pub struct ProcessRunner {
    bin: PathBuf,
}

impl ProcessRunner {
    pub fn new(bin: PathBuf) -> Self {
        Self { bin }
    }

    /// Locates yt-dlp, or `None` if it isn't installed.
    pub fn detect() -> Option<Self> {
        find_binary().map(Self::new)
    }

    pub fn bin(&self) -> &Path {
        &self.bin
    }

    /// A `Command` for the binary, with the console suppressed on Windows.
    ///
    /// This is a GUI-subsystem app, so every spawn of a console executable
    /// allocates a *visible* console window — one flash per resolved track,
    /// a whole cascade when a playlist prefetches. `CREATE_NO_WINDOW` keeps
    /// the child windowless; piped/captured stdio is unaffected.
    fn command(&self) -> Command {
        #[allow(unused_mut)]
        let mut cmd = Command::new(&self.bin);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }

    /// yt-dlp's version string (its own `--version` output, e.g. `2026.07.04`).
    pub fn version(&self) -> Option<String> {
        let out = self.command().arg("--version").output().ok()?;
        if !out.status.success() {
            return None;
        }
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!v.is_empty()).then_some(v)
    }
}

impl YtDlpRunner for ProcessRunner {
    fn run(&self, args: &[String]) -> Result<String, YtDlpError> {
        let out = self
            .command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => YtDlpError::NotInstalled,
                _ => YtDlpError::Failed(e.to_string()),
            })?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Print the invocation verbatim, because a failure here is only ever
        // reproducible if you know exactly what ran. The same command, by hand,
        // has succeeded while the app's copy failed — and with nothing logged
        // there was no way to tell whether the app really sends what this code
        // appears to send. Visible when the app is launched from a terminal.
        eprintln!(
            "[hm-ytmusic] yt-dlp exited {}\n  bin:    {}\n  args:   {:?}\n  stderr: {}",
            out.status,
            self.bin.display(),
            args,
            stderr.trim()
        );
        Err(classify(&stderr))
    }

    fn run_streaming(
        &self,
        args: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> Result<String, YtDlpError> {
        let mut child = self
            .command()
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => YtDlpError::NotInstalled,
                _ => YtDlpError::Failed(e.to_string()),
            })?;

        // Drain stderr on a helper thread: a download that fills the stderr pipe
        // while we're blocked reading stdout would deadlock.
        let stderr = child.stderr.take();
        let err_thread = std::thread::spawn(move || {
            let mut buf = String::new();
            if let Some(stderr) = stderr {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
            buf
        });

        let mut stdout_buf = String::new();
        if let Some(stdout) = child.stdout.take() {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                on_line(&line);
                stdout_buf.push_str(&line);
                stdout_buf.push('\n');
            }
        }

        let status = child
            .wait()
            .map_err(|e| YtDlpError::Failed(e.to_string()))?;
        let stderr_buf = err_thread.join().unwrap_or_default();

        if status.success() {
            Ok(stdout_buf)
        } else {
            Err(classify(&stderr_buf))
        }
    }
}

/// Maps yt-dlp's stderr onto an actionable error.
///
/// Split out (and tested) because the difference between "you need to sign in"
/// and "this track is gone" is the difference between a fixable prompt and a
/// dead end for the user.
pub fn classify(stderr: &str) -> YtDlpError {
    let low = stderr.to_lowercase();
    if low.contains("confirm you're not a bot")
        || low.contains("confirm your age")
        || low.contains("sign in to confirm")
        || low.contains("po token")
        || low.contains("http error 403")
    {
        return YtDlpError::Blocked(first_error_line(stderr));
    }
    // Two very different failures that used to share one (wrong) message: the
    // first means "formats exist, none of them m4a"; the second means the
    // extraction returned nothing at all, which is not a codec problem.
    if low.contains("requested format is not available") {
        return YtDlpError::NoCompatibleFormat(first_error_line(stderr));
    }
    if low.contains("no video formats found") {
        return YtDlpError::NoFormats(first_error_line(stderr));
    }
    if low.contains("video unavailable")
        || low.contains("private video")
        || low.contains("has been removed")
        || low.contains("not available in your country")
        || low.contains("this video is unavailable")
    {
        return YtDlpError::Unavailable(first_error_line(stderr));
    }
    YtDlpError::Failed(first_error_line(stderr))
}

/// Pulls the first `ERROR:` line out of yt-dlp's stderr, falling back to the
/// first non-empty line. Keeps multi-line warning spam out of the UI.
fn first_error_line(stderr: &str) -> String {
    stderr
        .lines()
        .find(|l| l.trim_start().starts_with("ERROR:"))
        .or_else(|| stderr.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("unknown error")
        .trim()
        .trim_start_matches("ERROR:")
        .trim()
        .to_string()
}

fn watch_url(video_id: &str) -> String {
    format!("https://music.youtube.com/watch?v={video_id}")
}

/// Base args every invocation shares.
///
/// Note what's deliberately absent: no `--extractor-args player_client=...`.
/// Pinning the `web_music` client seems natural for YT Music but makes it serve
/// *no* audio formats at all without a PO token, while yt-dlp's own default
/// client ladder returns itag 140 (m4a, 130 kbps) reliably. Choosing clients is
/// the job of the tool we're delegating to precisely because it tracks
/// YouTube's changes — overriding its defaults here only breaks that.
fn base_args(cookies: Option<&Path>) -> Vec<String> {
    let mut args = vec![
        "--no-warnings".to_string(),
        "--no-playlist".to_string(),
        "--no-progress".to_string(),
    ];
    if let Some(path) = cookies {
        args.push("--cookies".to_string());
        args.push(path.to_string_lossy().into_owned());
    }
    args
}

/// Resolves `video_id` into a directly-fetchable stream, without downloading.
///
/// `cookies` (when present) is what lets a Premium account skip the PO-token
/// requirement — the single most fragile dependency in this path.
pub fn resolve(
    runner: &dyn YtDlpRunner,
    video_id: &str,
    cookies: Option<&Path>,
) -> Result<StreamTarget, YtDlpError> {
    let mut args = base_args(cookies);
    args.push("-f".to_string());
    args.push(AUDIO_FORMAT.to_string());
    for field in PRINT_FIELDS {
        args.push("--print".to_string());
        args.push((*field).to_string());
    }
    args.push(watch_url(video_id));

    parse_resolve(&runner.run(&args)?)
}

/// Resolves `video_id` to its video-only rendition ([`VIDEO_FORMAT`]).
///
/// Deliberately a sibling of [`resolve`] rather than a flag on it: the audio
/// path is what playback depends on, and nothing about the picture may reach
/// into it. Callers treat a failure here as "no video", never as a play error.
pub fn resolve_video(
    runner: &dyn YtDlpRunner,
    video_id: &str,
    cookies: Option<&Path>,
) -> Result<StreamTarget, YtDlpError> {
    let mut args = base_args(cookies);
    args.push("-f".to_string());
    args.push(VIDEO_FORMAT.to_string());
    for field in PRINT_FIELDS {
        args.push("--print".to_string());
        args.push((*field).to_string());
    }
    args.push(watch_url(video_id));

    parse_resolve(&runner.run(&args)?)
}

/// Parses the `--print` block. Split out so the field-order contract with
/// [`PRINT_FIELDS`] is directly testable.
fn parse_resolve(stdout: &str) -> Result<StreamTarget, YtDlpError> {
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    if lines.len() < PRINT_FIELDS.len() {
        return Err(YtDlpError::Failed(format!(
            "expected {} lines from yt-dlp, got {}",
            PRINT_FIELDS.len(),
            lines.len()
        )));
    }

    let url = lines[0].to_string();
    if url.is_empty() || url == NA {
        // yt-dlp exited cleanly but gave no url. Report the format it *did*
        // name (also often NA): guessing at the reason is what made this error
        // untrustworthy, and the ext/format tell whoever reads the toast far
        // more than an invented explanation would.
        return Err(YtDlpError::NoCompatibleFormat(format!(
            "yt-dlp returned no stream url (selector {AUDIO_FORMAT:?}, it reported ext={:?} format={:?})",
            lines[1], lines[2]
        )));
    }
    let ext = lines[1].to_string();
    let format_id = lines[2].to_string();
    let abr_kbps = lines[3].parse::<f64>().ok().map(|v| v.round() as u32);

    // `%(http_headers)j` is a JSON object. Passing these through matters: the
    // CDN is picky about the User-Agent matching the client that resolved.
    let headers = serde_json::from_str::<HashMap<String, String>>(lines[4])
        .map(|m| {
            let mut v: Vec<(String, String)> = m.into_iter().collect();
            v.sort(); // deterministic order for tests and logging
            v
        })
        .unwrap_or_default();

    Ok(StreamTarget {
        expires_at: parse_expiry(&url),
        url,
        headers,
        ext,
        format_id,
        abr_kbps,
    })
}

/// A download's progress, as parsed from yt-dlp's progress template.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

impl Progress {
    pub fn fraction(&self) -> Option<f64> {
        let total = self.total_bytes?;
        (total > 0).then(|| (self.downloaded_bytes as f64 / total as f64).clamp(0.0, 1.0))
    }
}

/// Machine-readable progress: we drive our own template rather than parse
/// yt-dlp's human-facing bar, which is unstable across versions.
const PROGRESS_TEMPLATE: &str = "download:HMPROG %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s";

/// Marks the final path line so it can't be confused with progress output.
const PATH_PREFIX: &str = "HMPATH ";

/// Downloads `video_id` into `dest_dir` and returns the written file's path.
///
/// Tags and cover art are embedded when ffmpeg is available; without it the m4a
/// still downloads, just without embedded metadata, which the library scanner
/// tolerates (it falls back to the filename).
pub fn download(
    runner: &dyn YtDlpRunner,
    video_id: &str,
    dest_dir: &Path,
    cookies: Option<&Path>,
    have_ffmpeg: bool,
    mut on_progress: impl FnMut(Progress),
) -> Result<PathBuf, YtDlpError> {
    let mut args = base_args(cookies);
    args.push("-f".to_string());
    args.push(AUDIO_FORMAT.to_string());
    args.push("-o".to_string());
    args.push(
        dest_dir
            .join("%(artist,uploader)s - %(track,title)s [%(id)s].%(ext)s")
            .to_string_lossy()
            .into_owned(),
    );
    if have_ffmpeg {
        args.push("--embed-metadata".to_string());
        args.push("--embed-thumbnail".to_string());
    }
    // Skip work already on disk, and never leave a half-file behind claiming to
    // be a track: yt-dlp writes `.part` and renames on completion.
    args.push("--no-overwrites".to_string());
    args.push("--newline".to_string());
    args.push("--progress".to_string());
    args.push("--progress-template".to_string());
    args.push(PROGRESS_TEMPLATE.to_string());
    // `after_move` fires post-rename and post-postprocessing, so this is the
    // real final path rather than the `.part`.
    args.push("--print".to_string());
    args.push(format!("after_move:{PATH_PREFIX}%(filepath)s"));
    args.push(watch_url(video_id));

    let mut final_path: Option<PathBuf> = None;
    let stdout = runner.run_streaming(&args, &mut |line| {
        if let Some(rest) = line.strip_prefix(PATH_PREFIX) {
            final_path = Some(PathBuf::from(rest.trim()));
        } else if let Some(p) = parse_progress(line) {
            on_progress(p);
        }
    })?;

    // `--no-overwrites` on an existing file skips the download, and older yt-dlp
    // builds don't fire `after_move` in that case — recover the path from stdout.
    final_path
        .or_else(|| {
            stdout
                .lines()
                .find_map(|l| l.strip_prefix(PATH_PREFIX))
                .map(|p| PathBuf::from(p.trim()))
        })
        .ok_or_else(|| YtDlpError::Failed("yt-dlp reported no output file".to_string()))
}

/// Parses one `PROGRESS_TEMPLATE` line. `total_bytes` is `NA` for streams whose
/// length isn't known up front, in which case yt-dlp fills the estimate instead.
fn parse_progress(line: &str) -> Option<Progress> {
    let rest = line.strip_prefix("HMPROG ")?;
    let mut parts = rest.split_whitespace();
    let downloaded = parts.next()?.parse::<u64>().ok()?;
    let total = parts.next().and_then(|v| v.parse::<u64>().ok());
    let estimate = parts.next().and_then(|v| v.parse::<f64>().ok());
    Some(Progress {
        downloaded_bytes: downloaded,
        total_bytes: total.or_else(|| estimate.map(|v| v as u64)),
    })
}

/// Whether ffmpeg is reachable, which decides if downloads get embedded tags.
/// Checks the app-managed dir first for the same reason [`find_binary`] does;
/// yt-dlp itself finds a managed ffmpeg via its own same-directory lookup.
pub fn have_ffmpeg() -> bool {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Some(dir) = managed_bin_dir() {
        if is_executable(&dir.join(name)) {
            return true;
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if is_executable(&dir.join(name)) {
                return true;
            }
        }
    }
    EXTRA_DIRS
        .iter()
        .any(|d| is_executable(&Path::new(d).join(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        stdout: String,
    }
    impl YtDlpRunner for Fake {
        fn run(&self, _args: &[String]) -> Result<String, YtDlpError> {
            Ok(self.stdout.clone())
        }
    }

    fn sample_stdout() -> String {
        [
            "https://rr2---sn-abc.googlevideo.com/videoplayback?expire=1784124824&ip=1.2.3.4",
            "m4a",
            "140",
            "129.502",
            r#"{"User-Agent": "Mozilla/5.0", "Accept": "*/*"}"#,
        ]
        .join("\n")
    }

    #[test]
    fn parses_resolve_output() {
        let t = parse_resolve(&sample_stdout()).unwrap();
        assert!(t.url.starts_with("https://rr2---sn-abc.googlevideo.com/"));
        assert_eq!(t.ext, "m4a");
        assert_eq!(t.format_id, "140");
        assert_eq!(t.abr_kbps, Some(130));
        assert_eq!(
            t.headers,
            vec![
                ("Accept".to_string(), "*/*".to_string()),
                ("User-Agent".to_string(), "Mozilla/5.0".to_string()),
            ]
        );
    }

    #[test]
    fn reads_the_expiry_the_cdn_stamped_on_the_url() {
        let t = parse_resolve(&sample_stdout()).unwrap();
        assert_eq!(t.expires_at, Some(1784124824));
    }

    /// Shape captured from a live resolve: `expire` is neither first nor last,
    /// and several later parameters also end in the letters "expire".
    #[test]
    fn finds_expire_among_the_real_parameter_soup() {
        let url = "https://rr2---sn-n585oo54pcgx-xcce.googlevideo.com/videoplayback\
                   ?ei=zRpaauTIFPjjxN8P&expire=1784311597&ip=102.209.111.95&itag=140\
                   &mime=audio%2Fmp4&dur=213.089&lmt=1766955925572207&c=ANDROID_VR";
        assert_eq!(parse_expiry(url), Some(1784311597));
    }

    /// No deadline is not a long deadline. A caller that caches must be told
    /// "unknown" so it declines to cache, rather than inventing a lifetime.
    #[test]
    fn a_url_without_an_expiry_reports_none() {
        assert_eq!(parse_expiry("https://example.com/a.m4a"), None);
        assert_eq!(parse_expiry("https://example.com/a.m4a?itag=140"), None);
        assert_eq!(parse_expiry("https://example.com/a.m4a?expire=soon"), None);
    }

    /// `expire=` must match a whole parameter, not a suffix of one.
    #[test]
    fn does_not_mistake_a_similarly_named_parameter_for_the_expiry() {
        assert_eq!(parse_expiry("https://e.com/v?noexpire=123&itag=140"), None);
    }

    #[test]
    fn resolve_goes_through_runner() {
        let t = resolve(
            &Fake {
                stdout: sample_stdout(),
            },
            "dQw4w9WgXcQ",
            None,
        )
        .unwrap();
        assert_eq!(t.format_id, "140");
    }

    #[test]
    fn na_url_means_no_compatible_format() {
        let out = ["NA", "NA", "NA", "NA", "{}"].join("\n");
        let err = parse_resolve(&out).unwrap_err();
        assert!(
            matches!(err, YtDlpError::NoCompatibleFormat(_)),
            "got {err:?}"
        );
        // The reader gets the selector we asked with and what yt-dlp answered,
        // rather than a story about codecs we never checked.
        let shown = err.to_string();
        assert!(shown.contains("no stream url"), "got {shown:?}");
        assert!(shown.contains("bestaudio[ext=m4a]"), "got {shown:?}");
        assert!(!shown.contains("Opus"), "must not blame the codec: {shown:?}");
    }

    #[test]
    fn truncated_output_is_an_error() {
        assert!(matches!(
            parse_resolve("only-one-line").unwrap_err(),
            YtDlpError::Failed(_)
        ));
    }

    #[test]
    fn malformed_headers_degrade_to_empty() {
        let out = [
            "https://x/videoplayback",
            "m4a",
            "140",
            "128",
            "not-json",
        ]
        .join("\n");
        assert!(parse_resolve(&out).unwrap().headers.is_empty());
    }

    #[test]
    fn classifies_bot_check_as_blocked() {
        let e = classify("ERROR: [youtube] abc: Sign in to confirm you're not a bot");
        assert!(matches!(e, YtDlpError::Blocked(_)));
    }

    #[test]
    fn classifies_403_as_blocked() {
        assert!(matches!(
            classify("ERROR: unable to download video data: HTTP Error 403: Forbidden"),
            YtDlpError::Blocked(_)
        ));
    }

    #[test]
    fn classifies_missing_format() {
        let err = classify("ERROR: [youtube] abc: Requested format is not available");
        assert!(
            matches!(err, YtDlpError::NoCompatibleFormat(_)),
            "got {err:?}"
        );
        // yt-dlp's own line reaches the user.
        assert!(
            err.to_string().contains("Requested format is not available"),
            "got {err}"
        );
    }

    /// "No video formats found" is an empty extraction, not a codec problem, and
    /// must not claim the track is Opus-only. Reporting the two identically cost
    /// a long misdiagnosis: the track in question had itag 140 all along.
    #[test]
    fn an_empty_extraction_is_not_reported_as_opus_only() {
        let err = classify("ERROR: [youtube] abc: No video formats found!; please report this");
        assert!(
            matches!(err, YtDlpError::NoFormats(_)),
            "expected NoFormats, got {err:?}"
        );
        let shown = err.to_string();
        assert!(
            !shown.contains("Opus"),
            "must not blame the codec: {shown:?}"
        );
        // yt-dlp's own words survive, so the next report names the real cause.
        assert!(shown.contains("No video formats found"), "got {shown:?}");
    }

    #[test]
    fn classifies_unavailable() {
        assert!(matches!(
            classify("ERROR: [youtube] abc: Video unavailable"),
            YtDlpError::Unavailable(_)
        ));
    }

    #[test]
    fn error_line_strips_prefix_and_warnings() {
        let stderr = "WARNING: something noisy\nERROR: [youtube] abc: Video unavailable\n";
        match classify(stderr) {
            YtDlpError::Unavailable(m) => assert_eq!(m, "[youtube] abc: Video unavailable"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_progress_with_total() {
        let p = parse_progress("HMPROG 1024 4096 NA").unwrap();
        assert_eq!(p.downloaded_bytes, 1024);
        assert_eq!(p.total_bytes, Some(4096));
        assert_eq!(p.fraction(), Some(0.25));
    }

    #[test]
    fn parses_progress_falling_back_to_estimate() {
        let p = parse_progress("HMPROG 512 NA 2048.0").unwrap();
        assert_eq!(p.total_bytes, Some(2048));
    }

    #[test]
    fn progress_without_total_has_no_fraction() {
        let p = parse_progress("HMPROG 512 NA NA").unwrap();
        assert_eq!(p.total_bytes, None);
        assert_eq!(p.fraction(), None);
    }

    #[test]
    fn ignores_non_progress_lines() {
        assert!(parse_progress("[download] Destination: foo.m4a").is_none());
    }

    /// Exercises the real binary against the real CDN. Ignored by default so CI
    /// and offline builds stay green; run with
    /// `cargo test -p hm-ytmusic -- --ignored` to check the contract still holds
    /// after a yt-dlp or YouTube change.
    #[test]
    #[ignore = "requires yt-dlp on PATH and network access"]
    fn resolves_a_real_track_to_a_decodable_format() {
        let runner = ProcessRunner::detect().expect("yt-dlp not on PATH");
        let t = resolve(&runner, "dQw4w9WgXcQ", None).expect("resolve failed");

        assert!(t.url.contains("googlevideo.com"), "url was {}", t.url);
        // The decoder constraint, checked against reality: symphonia here reads
        // AAC-in-MP4 but not Opus/WebM, so anything else is silence.
        assert_eq!(t.ext, "m4a", "got a format the engine cannot decode");
        assert!(
            t.headers.iter().any(|(k, _)| k == "User-Agent"),
            "CDN needs the resolving client's User-Agent"
        );
    }

    #[test]
    fn format_selector_never_accepts_opus() {
        // Guards the decoder constraint: symphonia here has no Opus/WebM
        // support, so an Opus rendition would play as silence.
        assert!(AUDIO_FORMAT.contains("ext=m4a"));
        assert!(!AUDIO_FORMAT.contains("opus"));
        assert!(!AUDIO_FORMAT.contains("webm"));
    }

    /// The same constraint one layer up: the webview decodes H.264 everywhere
    /// and AV1 almost nowhere, and YouTube offers av01 at every rung — so an
    /// unpinned `bestvideo` selects a format that renders a blank frame and says
    /// nothing about it.
    #[test]
    fn video_selector_never_accepts_av1() {
        assert!(VIDEO_FORMAT.contains("vcodec^=avc1"));
        assert!(!VIDEO_FORMAT.contains("av01"));
        // Every alternative in the `/`-separated ladder must carry the pin —
        // a fallback without it would quietly reintroduce AV1.
        for alt in VIDEO_FORMAT.split('/') {
            assert!(alt.contains("vcodec^=avc1"), "unpinned alternative: {alt}");
            assert!(alt.contains("ext=mp4"), "non-mp4 alternative: {alt}");
        }
    }

    /// Muted-picture-only is the mechanism that keeps audio in the DSP chain.
    /// A rendition with an audio track would make bypassing it possible.
    #[test]
    fn video_selector_asks_only_for_video() {
        assert!(VIDEO_FORMAT.starts_with("bestvideo"));
        for alt in VIDEO_FORMAT.split('/') {
            assert!(alt.starts_with("bestvideo"), "not video-only: {alt}");
        }
    }

    /// Drops an executable fake named like the real binary into `dir`.
    fn plant_binary(dir: &Path) -> PathBuf {
        let p = dir.join(BIN_NAME);
        std::fs::write(&p, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    /// The managed copy is the one the app keeps updated, so it must win over
    /// anything PATH could offer.
    #[test]
    fn managed_dir_wins_over_path() {
        let tmp = std::env::temp_dir().join(format!("hm-ytdlp-managed-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let planted = plant_binary(&tmp);
        assert_eq!(find_binary_with(Some(&tmp)), Some(planted));
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// An empty managed dir must fall through to the usual lookup instead of
    /// masking a PATH install.
    #[test]
    fn empty_managed_dir_falls_through() {
        let tmp = std::env::temp_dir().join(format!("hm-ytdlp-empty-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        // Whatever the machine-wide answer is, an empty managed dir must not
        // change it.
        assert_eq!(find_binary_with(Some(&tmp)), find_binary_with(None));
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// A plain file that isn't executable must not count as an install (unix
    /// permission semantics; on Windows presence is the whole check).
    #[cfg(unix)]
    #[test]
    fn non_executable_managed_file_is_ignored() {
        let tmp = std::env::temp_dir().join(format!("hm-ytdlp-noexec-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join(BIN_NAME), b"").unwrap();
        assert_eq!(find_binary_with(Some(&tmp)), find_binary_with(None));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
