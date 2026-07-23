//! Streamed gapless / crossfade queue (cloud, phone, and YouTube Music).
//!
//! Mirrors [`QueuePlaybackSource`](crate::queue::QueuePlaybackSource) but decodes
//! each track by **streaming it over the network** rather than reading a local
//! file, and resolves each track's URL **lazily** — just before it's needed —
//! via a caller-supplied [`StreamResolver`]. That matters because some providers
//! cost real time per URL and hand out short-lived links: Dropbox an API call,
//! YouTube Music a whole yt-dlp process. Resolving the queue up front would be
//! slow and the links could go stale before they were reached.
//!
//! Only the current track and a one-track lookahead are held decoded in memory;
//! tracks already played are freed. So a long queue never downloads or decodes
//! everything at once. The lookahead is requested at each track boundary, giving
//! it a whole track's duration to resolve, download and decode.
//!
//! When it doesn't make it in time the crossfade is not abandoned — it ramps
//! over whatever frames remain (see `xf_len`). A ramp that has to be short is
//! still better than one cut off partway, and that case belongs to slow links,
//! which is where the seams show most.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use hm_core::TrackMeta;

use crate::decode::{
    decode_file, decode_format_chunked, open_format_stream, resample_stereo, DecodeChunk,
    StreamResampler,
};
use crate::engine::MetaSink;
use crate::error::AudioError;
use crate::queue::write_frame;
use crate::{AudioSource, StreamFormat};

/// Where to fetch one streamed track from.
#[derive(Clone, Debug)]
pub struct StreamTarget {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// File-extension hint (e.g. `"mp3"`) so the demuxer picks the right reader.
    pub ext: Option<String>,
}

/// Resolves the [`StreamTarget`] for absolute queue position `i`, lazily. An
/// `Err` (or a later decode failure) turns that track into a silent skip rather
/// than stopping the queue.
///
/// `fresh` says the last target for `i` didn't work, so a resolver holding one
/// must not hand back the same answer. Providers whose links are dated can cache
/// them, but a link may still die before its stated deadline — googlevideo binds
/// each to the address that resolved it, and nothing in the url marks that. Only
/// the caller learns it failed, so only the caller can say so; without this a
/// cache would replay a dead link across every attempt and make one transient
/// failure permanent.
pub type StreamResolver = Arc<dyn Fn(usize, bool) -> Result<StreamTarget, String> + Send + Sync>;

/// The state of one absolute-index track slot in [`StreamQueueSource::tracks`].
///
/// Replaces the old `Option<Vec<f32>>` (`None` / `Some(empty)` / `Some(samples)`)
/// so a track can be *playing* before it's *finished* decoding: `Growing` holds
/// whatever PCM has arrived so far and keeps accepting more via [`DecodeEvent::Chunk`];
/// `Done` is the terminal state (a track that failed or was skipped is
/// `Done(Vec::new())` — silence that's still eligible to advance past, exactly
/// like the old `Some(empty)`).
#[derive(Debug)]
enum Slot {
    /// Not decoded at all yet — buffer silence, don't advance past it.
    Empty,
    /// Partially decoded; more [`DecodeEvent::Chunk`]s may still extend this.
    Growing(Vec<f32>),
    /// Fully decoded (or permanently failed/empty) — safe to advance past once
    /// the cursor reaches its end.
    Done(Vec<f32>),
}

/// One update from the decode worker to the read-side [`Slot`] state machine.
///
/// A track's PCM arrives over several `Chunk`s as the worker decodes it while
/// still downloading it (see `spawn`'s inner loop), letting the current track
/// start playing on partial data (see [`StreamQueueSource::ready`]) instead of
/// waiting for the whole file.
enum DecodeEvent {
    /// Tag metadata arrived — apply eagerly so the UI can show it before the
    /// track is playable. `capacity_frames`, when known, estimates the
    /// track's total decoded-and-resampled size (interleaved stereo `f32`
    /// element count, at `device_rate`) from the container's declared frame
    /// count — a one-time `reserve_exact` hint so the growing buffer doesn't
    /// creep up via repeated reallocation as `Chunk`s arrive (see `drain`'s
    /// `Meta` arm). `None` when the container didn't declare a frame count
    /// up front; the buffer still grows correctly, just via `Vec`'s own
    /// amortized-growth reallocation instead of one reservation.
    Meta {
        idx: usize,
        meta: TrackMeta,
        capacity_frames: Option<usize>,
    },
    /// More decoded PCM for `idx`: `Empty` becomes `Growing`, `Growing`
    /// extends — except a *genuinely fresh* `Growing` (zero length AND zero
    /// capacity: brand new, or just emptied by a `Reset`), which is replaced
    /// by the incoming `Vec` outright (a move, not a copy) rather than
    /// extended into, avoiding a pointless allocate-then-copy on the audio
    /// thread for the common first-chunk / post-`Reset`-republish case. An
    /// empty `Growing` that already has capacity — a `Meta` capacity hint
    /// reserved it (see `capacity_frames`) — still extends, so that
    /// reservation isn't discarded by the move. A `Chunk` after `Done` is a
    /// protocol bug (the worker moved on) — ignored in release,
    /// `debug_assert`ed in tests/dev builds.
    Chunk { idx: usize, samples: Vec<f32> },
    /// No more PCM is coming for `idx` — `Growing` becomes `Done`, and an
    /// `Empty` slot (nothing ever arrived) becomes `Done(Vec::new())`, i.e.
    /// the old "decoded but empty" skip.
    Done { idx: usize },
    /// The track failed permanently — `Done(Vec::new())`, same as `Done` on an
    /// `Empty` slot: a silent, instantly-skippable track.
    Failed { idx: usize },
    /// Drop whatever PCM was buffered for `idx` and go back to `Growing(empty)`
    /// — sent when a track needs to be redecoded: a truncated-EOF or
    /// mid-stream decode failure that already published some `Chunk`s, right
    /// before either a retry or the whole-spool-file fallback republishes
    /// from zero. The read side's cursor is untouched, so playback stalls
    /// exactly where it was rather than restarting from the top.
    Reset { idx: usize },
}

/// How many times the worker tries to fetch + decode one track before giving up
/// and skipping it. Transient failures — a connection dropped while the stream
/// sat paused, a flaky cloud link, a phone that closed its keep-alive, a 5xx —
/// are retried; reqwest/hyper evicts an errored connection from the pool, so
/// when the old connection was the problem, the retry dials fresh without this
/// code needing to force it. A permanent failure (404/403, an undecodable
/// body) is skipped at once without burning the retry budget. This is what
/// stops one stale connection from silently nuking a good track (which looked
/// like the queue "jumping" or "stopping" on its own).
const MAX_ATTEMPTS: u32 = 4;

/// Total time budget for one lookahead download (`open_stream`'s `req.send()`),
/// covering connect through reading the whole body. This can't live on the
/// client itself: the client is [`streaming::shared_client`], also used by
/// `streaming.rs`'s progressive playback path, whose reads legitimately
/// outlive any one track's download — a client-level cap would sever that
/// stream mid-song. Applied per-request instead, it only ever bounds a
/// one-shot, complete-in-one-call fetch, where a stuck download (e.g. a dead
/// pooled connection, common after the stream has sat paused a while) fails
/// fast and is retried rather than stalling the queue on a long hang.
const FETCH_TIMEOUT: Duration = Duration::from_secs(90);

/// Minimum **source-rate** frames [`decode_format_chunked`] accumulates
/// before flushing a `Pcm` chunk to the worker's sink — about a second at a
/// typical 48 kHz source rate. Not tied to the device rate: each source-rate
/// chunk is resampled to the device rate as it arrives (see
/// [`StreamResampler`]), so a ~1 s source chunk becomes a ~1 s device-rate
/// `Chunk` — matching the read side's own ~1 s start/crossfade-trust gate
/// (`start_frames`) closely enough that the first chunk clears it almost as
/// soon as it lands, without flooding the channel with many tiny sends.
///
/// Being a fixed **source-rate** frame count, the minimum flush spans
/// `48_000 / src_rate` seconds of audio — more than a second for a source
/// rate below 48 kHz. Harmless: the bytes involved stay small either way, so
/// a slower-than-1 s first flush doesn't change the channel-flooding math
/// above.
const CHUNK_FRAMES: usize = 48_000;

/// A downloaded track body spooled to a temp file rather than held in RAM (a
/// long lossless track can be hundreds of MB compressed — and the old
/// buffer-then-copy path held it twice). The file is removed when this guard
/// drops, so every exit path — decode success, decode failure, a retried
/// truncated download, or the whole source being dropped — cleans up.
struct SpooledBody {
    path: PathBuf,
}

impl SpooledBody {
    /// Create a unique spool file for one download attempt, keeping the track's
    /// extension (sanitized) so the demuxer probe gets the same format hint
    /// `decode_bytes` used to receive.
    fn create(ext: Option<&str>) -> std::io::Result<(Self, std::fs::File)> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let mut name = format!(
            "hm-stream-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        if let Some(ext) = ext {
            let safe: String = ext
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(8)
                .collect();
            if !safe.is_empty() {
                name.push('.');
                name.push_str(&safe);
            }
        }
        let path = std::env::temp_dir().join(name);
        let file = std::fs::File::create(&path)?;
        Ok((Self { path }, file))
    }
}

impl Drop for SpooledBody {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The outcome of opening the network connection for one track — before any
/// of the body has been read. Classified exactly like the old `fetch_once`
/// (whose body-copy step moved into the worker below, which now streams the
/// body straight into the decoder instead of buffering it first).
enum Opened {
    /// Connected; ready to stream the body. `declared` is the server's
    /// stated length (`Content-Length`), when given — used later to detect
    /// a truncated download (see [`after_stream_failure`]/[`finish_or_retry`]).
    Body { resp: reqwest::blocking::Response, declared: Option<u64> },
    /// Permanent failure (4xx) — retrying the same URL won't help; skip it.
    Skip,
    /// Transient failure (connect/read error, timeout, 408/429/5xx) — worth
    /// retrying on a fresh connection.
    Retry,
}

/// Issue one GET for `target` and classify the response status, without
/// reading any of the body — that now happens streamed, straight into the
/// decoder (see [`TeeReader`] and the worker in `spawn`).
fn open_stream(client: &reqwest::blocking::Client, target: &StreamTarget) -> Opened {
    let mut req = client.get(&target.url);
    for (k, v) in &target.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    // Ask for the whole thing *as a range*.
    //
    // googlevideo serves a plain GET at roughly the track's own bitrate — it is
    // pacing a player it assumes is listening in real time. Measured on one
    // 3.4 MB track: 106s for the plain GET, 0.56s for the identical request
    // carrying this header. Nothing else differs, and `bytes=0-` asks for no
    // less than the whole body; the header alone is what opts out of the pacing.
    //
    // It matters most here of anywhere. This path downloads a track *before* it
    // can play a note of it, so the throttle wasn't costing throughput, it was
    // costing the whole wait — and a lookahead that takes a track's own duration
    // to arrive can never be ready early, which is exactly what a crossfade
    // needs it to be.
    req = req.header(reqwest::header::RANGE, "bytes=0-");
    // Per-request, not client-level: see [`FETCH_TIMEOUT`] for why.
    req = req.timeout(FETCH_TIMEOUT);
    match req.send() {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                let declared = resp.content_length();
                Opened::Body { resp, declared }
            } else if status.is_server_error()
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::REQUEST_TIMEOUT
            {
                Opened::Retry
            } else {
                Opened::Skip
            }
        }
        // Connect/timeout/transport error — the pooled connection may be stale.
        Err(_) => Opened::Retry,
    }
}

/// Reads through to `inner`, mirroring every byte into `file` (the track's
/// spool) and counting how many have passed through so far in `*seen`.
///
/// Holds `inner`/`file`/`seen` by mutable reference rather than by value: the
/// worker wraps a `TeeReader` around its response and spool file only for as
/// long as the streaming decoder is reading through it, and drops it (by
/// returning from a nested scope, not by holding onto it) the moment that
/// decode attempt ends — success, truncation, or a decode error alike. Once
/// dropped, these borrows release and the worker gets `resp`/`file`/`seen`
/// straight back to keep reading from exactly where the tee left off: the
/// fallback path (a mid-stream decode error with the body not yet fully
/// drained) finishes copying the rest of the response into the very same
/// spool file before decoding it whole.
struct TeeReader<'a, R> {
    inner: &'a mut R,
    file: &'a mut std::fs::File,
    seen: &'a mut u64,
}

impl<R: std::io::Read> std::io::Read for TeeReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            std::io::Write::write_all(self.file, &buf[..n])?;
            *self.seen += n as u64;
        }
        Ok(n)
    }
}

/// What to do once a streaming decode has ended in `Err` (a probe failure or
/// a mid-packet decode error — either way, the streaming path couldn't
/// finish this track).
enum StreamFailure {
    /// The body downloaded completely (the tee saw every declared byte, or
    /// there was no declared length to check against) — the spool is (or,
    /// after draining the rest of it, will be) a complete file. Decode it
    /// whole, exactly the old path: this rides out a container that needed
    /// seeking (streaming demux can't) or a mid-stream hiccup the streaming
    /// decoder couldn't ride out but a fresh whole-file pass can.
    DecodeSpool,
    /// The body stopped short — a transport problem, not a bad file. Retry
    /// on a fresh connection rather than decode a clipped track.
    Retry,
}

/// Classify a streaming-decode `Err` using how much of the declared body has
/// been seen. Same predicate as [`finish_or_retry`], asked on the decode-`Err`
/// path instead of decode-`Ok`.
///
/// The caller (see [`drain_then_classify`]) always calls this with the
/// **post-drain** byte count, not the count at the moment the decode/probe
/// error happened — a decode/probe error partway through an incomplete body
/// doesn't by itself mean the body is incomplete *for good*: the container
/// may simply need bytes the streaming decoder hasn't read yet (a
/// non-faststart mp4/m4a, `moov` at the tail — the demuxer gives up as soon
/// as it can't make progress without seeking, often after only a few KB, long
/// before the body is anywhere near fully downloaded). Classifying on that
/// pre-drain count would call a perfectly healthy download `Retry` and burn
/// the whole retry ladder on a transport that was never the problem.
fn after_stream_failure(bytes_seen: u64, declared: Option<u64>) -> StreamFailure {
    if finish_or_retry(bytes_seen, declared) {
        StreamFailure::DecodeSpool
    } else {
        StreamFailure::Retry
    }
}

/// Finish draining a failed streaming decode's response before classifying
/// it, rather than classifying on the byte count at the moment of failure.
///
/// `drain` does the (possibly slow, but bounded by the same [`FETCH_TIMEOUT`]
/// already on the response) work of reading whatever's left of the body and
/// returns the resulting **total** byte count; [`after_stream_failure`] then
/// classifies from that. Only a drain that itself comes up short — the
/// connection actually died — still falls through to `Retry`; a container
/// that merely needed the rest of the file to parse gets its fair shot at
/// the whole-file fallback instead of being retried to exhaustion for no
/// reason.
fn drain_then_classify(declared: Option<u64>, drain: impl FnOnce() -> u64) -> StreamFailure {
    after_stream_failure(drain(), declared)
}

/// Whether a clean end-of-stream (no decode error; the format reader simply
/// ran out of packets) can be trusted as a *real* end of track.
///
/// A clean EOF looks identical whether the whole body arrived or a server
/// simply stopped sending early — so this checks the tee's own byte count
/// against the server-declared length rather than trusting the decoder's
/// silence. `false` means: don't publish a short `Done`, retry instead. With
/// no declared length there's nothing to compare against, so this — like the
/// pre-streaming `is_none_or` truncation check it replaces — leans lenient
/// and calls it finished; a provider that never sends `Content-Length` would
/// otherwise never be trusted at all.
fn finish_or_retry(bytes_seen: u64, declared: Option<u64>) -> bool {
    declared.is_none_or(|n| bytes_seen >= n)
}

/// Whether a clean end-of-stream can be trusted as a real end of track,
/// draining the response to completion first if the byte count alone
/// doesn't already prove it — symmetric to [`drain_then_classify`]'s
/// reasoning on the decode-`Err` side, and for the same underlying reason:
/// a demuxer can reach a clean `Ok` end-of-packets before the whole body has
/// been read. RIFF/WAV (and AIFF) readers in particular stop as soon as
/// they've consumed the `data` chunk's declared length and never read
/// whatever comes after it — a trailing `LIST`/`INFO` chunk, embedded art,
/// an id3 chunk some encoders append — even though the container and every
/// audio frame in it are completely intact. Treating that short byte count
/// as truncation would retry (and eventually skip) a perfectly playable
/// file, which is exactly the regression class [`after_stream_failure`]'s
/// post-drain reclassification exists to close on the `Err` side — this
/// closes the equivalent gap on the `Ok` side.
///
/// `drain` is only invoked (via short-circuiting `||`) when `bytes_seen`
/// doesn't already prove completeness, so the common case (the tee already
/// saw everything) never pays for the extra read.
fn finish_or_drain_then_retry(
    bytes_seen: u64,
    declared: Option<u64>,
    drain: impl FnOnce() -> u64,
) -> bool {
    finish_or_retry(bytes_seen, declared) || finish_or_retry(drain(), declared)
}

/// One attempt's classified result. Unlike the old whole-track `Ready(...)`,
/// a successful attempt doesn't hand back decoded samples here — it already
/// published them (`Meta`/`Chunk`/`Done`, or `Reset` + `Meta`/`Chunk`/`Done`
/// for the whole-file fallback) straight to the read side as it went.
enum LoadAttempt {
    /// A terminal event (`Done` or `Failed`) was already sent for this track.
    Published,
    /// Give up on this track now (permanent failure) — no retry budget
    /// spent; the caller still owes it a terminal `Failed`.
    Skip,
    /// Transient failure — try again.
    Retry,
}

/// Whether [`load_with_retry`] itself already published every track's
/// terminal event, or gave up without ever doing so.
enum LadderOutcome {
    /// The winning attempt already sent `Done`/`Failed`.
    Published,
    /// A permanent `Skip`, an exhausted retry budget, or a stop request —
    /// none of those attempts publish anything themselves, so the caller
    /// must still send a terminal `Failed` (the same "gave up" outcome the
    /// old whole-track worker spelled as an empty decoded track).
    GaveUp,
}

/// Run `attempt` up to [`MAX_ATTEMPTS`] times, sleeping `backoff` (doubling, to
/// a 2 s cap) between transient retries. `backoff` is a parameter so tests can
/// pass zero. Publishing to the read side is entirely `attempt`'s own job (see
/// [`LoadAttempt`]) — this function only drives the attempts/backoff/stop
/// ladder, unchanged from before this task.
fn load_with_retry(
    running: &AtomicBool,
    backoff: Duration,
    mut attempt: impl FnMut(u32) -> LoadAttempt,
) -> LadderOutcome {
    let mut wait = backoff;
    for n in 1..=MAX_ATTEMPTS {
        if !running.load(Ordering::Relaxed) {
            break;
        }
        match attempt(n) {
            LoadAttempt::Published => return LadderOutcome::Published,
            LoadAttempt::Skip => break,
            LoadAttempt::Retry => {
                if n == MAX_ATTEMPTS {
                    break;
                }
                std::thread::sleep(wait);
                wait = (wait * 2).min(Duration::from_secs(2));
            }
        }
    }
    LadderOutcome::GaveUp
}

/// A queue of streamed tracks played gaplessly, with optional crossfade.
pub struct StreamQueueSource {
    /// Decoded tracks by absolute index — see [`Slot`]. Tracks below the play
    /// head are freed back to `Slot::Empty`.
    tracks: Vec<Slot>,
    metas: Vec<TrackMeta>,
    count: usize,
    device_rate: u32,
    crossfade: Arc<AtomicU32>,
    index: usize,
    cursor: usize,
    xf_cursor: usize,
    /// Frames the in-progress crossfade ramps over, latched when it starts;
    /// 0 when not crossfading.
    ///
    /// Not simply `crossfade_frames()`: a lookahead that finishes decoding after
    /// the window has already opened leaves fewer frames than that, and ramping
    /// against the full width would leave the outgoing track at audible gain
    /// when it's cut. Which is to say the click would land exactly when the
    /// network is slowest — the case crossfade most needs to survive.
    xf_len: usize,
    rx: Option<Receiver<DecodeEvent>>,
    /// Asks the worker to ensure tracks are decoded up to (and incl.) this index.
    want_tx: Option<Sender<usize>>,
    running: Arc<AtomicBool>,
    meta_sink: Option<MetaSink>,
    current_index: Option<Arc<AtomicUsize>>,
    _worker: Option<JoinHandle<()>>,
}

/// How long a `Growing` track must have played out on disk before it's
/// trusted as the *current* track (see [`StreamQueueSource::ready`]) or as a
/// crossfade's incoming track (see [`StreamQueueSource::growing_head_at_least`]).
/// One second: short enough that starting playback doesn't feel delayed,
/// long enough that ordinary jitter in a still-downloading track doesn't
/// immediately underrun into audible silence right after it starts.
const START_FRAMES_SECS: f32 = 1.0;

/// [`START_FRAMES_SECS`] in frames at the device's sample rate `rate` — the
/// rate everything in `tracks` is already resampled to, so this doesn't need
/// to know a track's own original rate.
fn start_frames(rate: u32) -> usize {
    (rate as f32 * START_FRAMES_SECS).round() as usize
}

/// Estimate the total interleaved-stereo `f32` element count a track will
/// occupy once fully decoded and resampled to `device_rate`, from the
/// container's declared source-rate frame count (see [`DecodeChunk::Meta`]) —
/// a one-time capacity hint for `drain`'s `Meta` arm (see
/// [`DecodeEvent::Meta`]) so the growing PCM buffer can be reserved once up
/// front instead of creeping up through repeated reallocation as `Chunk`s
/// arrive. Padded 2% over the raw rate-converted estimate: the resampler's
/// actual output count can land a hair either side of it (rounding at each
/// end of the conversion), and reserving slightly short would just mean the
/// first `extend` past the hint reallocates anyway — this only needs to be
/// close, not exact.
fn capacity_hint(num_frames: Option<u64>, src_rate: u32, device_rate: u32) -> Option<usize> {
    let frames = num_frames?;
    if src_rate == 0 {
        return None;
    }
    let device_frames = frames as f64 * device_rate as f64 / src_rate as f64;
    Some(((device_frames * 2.0) * 1.02).ceil() as usize) // * 2: interleaved stereo
}

/// Stream-decode one opened response body for track `idx`, publishing
/// `Meta`/`Chunk` events as PCM becomes available so the queue can start
/// playing long before the whole file has downloaded (see [`CHUNK_FRAMES`]).
///
/// Always tees the body to a spool file as it reads (see [`TeeReader`]), so a
/// streaming-decode failure can fall back to decoding the completed spool
/// whole — today's pre-streaming path, which rides out a container that
/// needed seeking (streaming demux can't) or a mid-stream hiccup the
/// streaming decoder couldn't. A decode/probe `Err` always finishes draining
/// the response into the spool *before* deciding whether that fallback
/// applies (see [`drain_then_classify`]) — an incomplete body at the moment
/// of the error doesn't mean the stream is unhealthy, only that the
/// container needed bytes the streaming decoder hadn't read yet. Every
/// branch that reaches a final outcome for this track sends exactly one
/// terminal event (`Done` or `Failed`); a `Reset` goes out first whenever
/// it's discarding `Chunk`s already published by this same call (an
/// abandoned partial decode being retried or superseded by the whole-file
/// fallback).
fn stream_decode_attempt(
    idx: usize,
    mut resp: reqwest::blocking::Response,
    declared: Option<u64>,
    ext: Option<&str>,
    device_rate: u32,
    tx: &Sender<DecodeEvent>,
    running: &AtomicBool,
) -> LoadAttempt {
    let (spool, mut file) = match SpooledBody::create(ext) {
        Ok(v) => v,
        Err(_) => return LoadAttempt::Retry,
    };

    let mut seen: u64 = 0;
    let mut resampler: Option<StreamResampler> = None;
    let mut meta_sent = false;
    let mut chunks_sent = false;

    // Scoped so the tee's borrows of `resp`/`file`/`seen` release the moment
    // this decode attempt ends (`format`, owning the tee, is dropped at the
    // end of this block) — every branch below needs them back directly.
    let decode_result = {
        let tee = TeeReader { inner: &mut resp, file: &mut file, seen: &mut seen };
        match open_format_stream(tee, ext) {
            Ok(format) => decode_format_chunked(format, CHUNK_FRAMES, &mut |chunk| {
                match chunk {
                    DecodeChunk::Meta(meta, src_rate, num_frames) => {
                        resampler = Some(StreamResampler::new(src_rate, device_rate));
                        meta_sent = true;
                        let capacity_frames = capacity_hint(num_frames, src_rate, device_rate);
                        let _ = tx.send(DecodeEvent::Meta { idx, meta, capacity_frames });
                    }
                    DecodeChunk::Pcm(pcm) => {
                        if let Some(r) = resampler.as_mut() {
                            let out = r.push(&pcm);
                            if !out.is_empty() {
                                chunks_sent = true;
                                let _ = tx.send(DecodeEvent::Chunk { idx, samples: out });
                            }
                        }
                    }
                }
                // Checked after Meta too, not just Pcm — teardown must abort
                // before the very first packet is even read, not only
                // mid-download.
                running.load(Ordering::Relaxed)
            }),
            Err(e) => Err(e),
        }
    };

    match decode_result {
        Ok(()) => {
            if !running.load(Ordering::Relaxed) {
                // Torn down mid-decode: don't finish *or* really retry, just
                // unwind — `load_with_retry`'s own running-check breaks the
                // ladder on its very next iteration.
                if chunks_sent {
                    let _ = tx.send(DecodeEvent::Reset { idx });
                }
                return LoadAttempt::Retry;
            }
            // A short byte count here doesn't retry immediately: it might be
            // unread trailing bytes (see `finish_or_drain_then_retry`), not a
            // truncated transport. Only a drain that itself comes up short
            // means a real truncation.
            let complete = finish_or_drain_then_retry(seen, declared, || {
                {
                    let mut tee =
                        TeeReader { inner: &mut resp, file: &mut file, seen: &mut seen };
                    let _ = std::io::copy(&mut tee, &mut std::io::sink());
                }
                seen
            });
            if complete {
                if let Some(r) = resampler.as_mut() {
                    let tail = r.finish();
                    if !tail.is_empty() {
                        chunks_sent = true;
                        let _ = tx.send(DecodeEvent::Chunk { idx, samples: tail });
                    }
                }
                if chunks_sent {
                    let _ = tx.send(DecodeEvent::Done { idx });
                } else {
                    // Clean EOF, complete body, but nothing ever decoded — an
                    // undecodable file, same skip as today's empty-Ready case.
                    let _ = tx.send(DecodeEvent::Failed { idx });
                }
                LoadAttempt::Published
            } else {
                // Still short even after draining to completion: a genuine
                // truncation, not a real end of track — never publish a
                // short `Done`.
                if chunks_sent {
                    let _ = tx.send(DecodeEvent::Reset { idx });
                }
                LoadAttempt::Retry
            }
        }
        Err(_) => {
            if !running.load(Ordering::Relaxed) {
                // Torn down before even attempting recovery — don't spend
                // the drain's up-to-`FETCH_TIMEOUT` network wait, nor
                // (below) the whole-spool decode's CPU time, on a channel
                // nobody's reading from anymore.
                if chunks_sent {
                    let _ = tx.send(DecodeEvent::Reset { idx });
                }
                return LoadAttempt::Retry;
            }
            match drain_then_classify(declared, || {
                // Finish draining the response into the SAME spool file: the
                // tee already mirrored everything read up to the failure, so
                // this just reads (and discards — it's already on disk)
                // whatever's left, leaving the spool a complete file —
                // *before* classifying (see `drain_then_classify`): a
                // probe/decode error on an incomplete body doesn't yet mean
                // the body can't be completed, only that the streaming
                // decoder couldn't make progress with what it had so far.
                {
                    let mut tee =
                        TeeReader { inner: &mut resp, file: &mut file, seen: &mut seen };
                    let _ = std::io::copy(&mut tee, &mut std::io::sink());
                }
                seen
            }) {
                StreamFailure::Retry => {
                    // The drain itself came up short — a genuine transport
                    // failure (the connection actually died), not a
                    // container that merely needed the rest of the file.
                    if chunks_sent {
                        let _ = tx.send(DecodeEvent::Reset { idx });
                    }
                    LoadAttempt::Retry
                }
                StreamFailure::DecodeSpool => {
                    if !running.load(Ordering::Relaxed) {
                        // Torn down during the drain — the spool sitting
                        // there (complete, or as complete as it'll get)
                        // still isn't worth spending real CPU decoding whole
                        // for a track nobody will ever hear; unwind the
                        // same way.
                        if chunks_sent {
                            let _ = tx.send(DecodeEvent::Reset { idx });
                        }
                        return LoadAttempt::Retry;
                    }
                    match decode_file(&spool.path) {
                        Ok(d) => {
                            let out = resample_stereo(&d.samples, d.sample_rate, device_rate);
                            if out.is_empty() {
                                if chunks_sent {
                                    let _ = tx.send(DecodeEvent::Reset { idx });
                                }
                                let _ = tx.send(DecodeEvent::Failed { idx });
                            } else {
                                if chunks_sent {
                                    // The whole-file decode republishes from
                                    // zero — without this Reset the track
                                    // would be doubled (partial streamed
                                    // PCM, then the whole file again).
                                    let _ = tx.send(DecodeEvent::Reset { idx });
                                }
                                if !meta_sent {
                                    // No incremental growth follows (`out`
                                    // below is the whole track in one
                                    // `Chunk`, then `Done`) — no capacity
                                    // hint to give.
                                    let _ = tx.send(DecodeEvent::Meta {
                                        idx,
                                        meta: d.meta,
                                        capacity_frames: None,
                                    });
                                }
                                let _ = tx.send(DecodeEvent::Chunk { idx, samples: out });
                                let _ = tx.send(DecodeEvent::Done { idx });
                            }
                        }
                        Err(_) => {
                            if chunks_sent {
                                let _ = tx.send(DecodeEvent::Reset { idx });
                            }
                            let _ = tx.send(DecodeEvent::Failed { idx });
                        }
                    }
                    LoadAttempt::Published
                }
            }
        }
    }
}

impl StreamQueueSource {
    /// Spawn a background streamer/decoder for `count` tracks resolved via
    /// `resolver`, playing from `start`. Holds only the current + next track
    /// decoded. Reports progress via `meta_sink` / `current_index`.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        resolver: StreamResolver,
        count: usize,
        start: usize,
        device_rate: u32,
        crossfade: Arc<AtomicU32>,
        meta_sink: Option<MetaSink>,
        current_index: Option<Arc<AtomicUsize>>,
    ) -> Self {
        let start = start.min(count.saturating_sub(1));
        let (tx, rx) = mpsc::channel::<DecodeEvent>();
        let (want_tx, want_rx) = mpsc::channel::<usize>();
        let running = Arc::new(AtomicBool::new(true));

        let worker = {
            let running = running.clone();
            std::thread::Builder::new()
                .name("hm-stream-queue".into())
                .spawn(move || {
                    // The process-wide shared client (see
                    // `streaming::shared_client`) — reused across tracks, and
                    // across this queue and the progressive-playback path
                    // alike, so consecutive tracks reuse the same connection
                    // pool instead of a fresh TLS handshake each time.
                    let client = crate::streaming::shared_client().clone();
                    let mut next = start;
                    while let Ok(want) = want_rx.recv() {
                        while next <= want && next < count {
                            if !running.load(Ordering::Relaxed) {
                                return;
                            }
                            let idx = next;
                            // Retry transient failures, so a dropped link — e.g.
                            // the first fetch after the stream sat paused, or a
                            // phone that closed its keep-alive — doesn't turn a
                            // good track into a permanent silent skip. Only a
                            // retry demands a fresh link: asking for one on the
                            // first attempt would make every provider that can
                            // cache pay to resolve anyway, and for YT Music
                            // resolving is a yt-dlp process start measured in
                            // seconds — the gap between two tracks.
                            //
                            // Publishing happens inside `stream_decode_attempt`
                            // itself (see `LoadAttempt`) — it decodes while
                            // still downloading, so a `Chunk` can reach the read
                            // side well before this whole ladder returns.
                            let outcome = load_with_retry(
                                &running,
                                Duration::from_millis(120),
                                |n| match resolver(idx, n > 1) {
                                    Err(_) => LoadAttempt::Retry,
                                    Ok(target) => match open_stream(&client, &target) {
                                        Opened::Skip => LoadAttempt::Skip,
                                        Opened::Retry => LoadAttempt::Retry,
                                        Opened::Body { resp, declared } => stream_decode_attempt(
                                            idx,
                                            resp,
                                            declared,
                                            target.ext.as_deref(),
                                            device_rate,
                                            &tx,
                                            &running,
                                        ),
                                    },
                                },
                            );
                            if let LadderOutcome::GaveUp = outcome {
                                // Nothing published a terminal event for this
                                // track (a permanent `Skip`, an exhausted
                                // retry budget, or a stop request) — send the
                                // uniform "gave up" event ourselves, the same
                                // silent-skip meaning the old whole-track
                                // worker's empty decoded track carried.
                                if tx.send(DecodeEvent::Failed { idx }).is_err() {
                                    return;
                                }
                            }
                            next += 1;
                        }
                    }
                })
                .ok()
        };

        let mut tracks = Vec::with_capacity(count);
        tracks.resize_with(count, || Slot::Empty);
        let metas = vec![TrackMeta::default(); count];

        // Prime the current track + one lookahead.
        let _ = want_tx.send(start + 1);

        Self {
            tracks,
            metas,
            count,
            device_rate,
            crossfade,
            index: start,
            cursor: 0,
            xf_cursor: 0,
            xf_len: 0,
            rx: Some(rx),
            want_tx: Some(want_tx),
            running,
            meta_sink,
            current_index,
            _worker: worker,
        }
    }

    /// Whether track `i` can become the *current* playing track: a `Done`
    /// track always can (even empty — that's the failed/skip case, eligible
    /// to advance straight past); a `Growing` one only once it's buffered
    /// [`start_frames`] worth, so playback doesn't start only to immediately
    /// underrun on ordinary download jitter; `Empty` never can.
    fn ready(&self, i: usize) -> bool {
        match self.tracks.get(i) {
            Some(Slot::Done(_)) => true,
            Some(Slot::Growing(s)) => s.len() / 2 >= start_frames(self.device_rate),
            _ => false,
        }
    }

    /// Whether track `i` is fully decoded (or a terminal failure/skip) — the
    /// only state from which a boundary/crossfade may advance *past* it.
    fn done(&self, i: usize) -> bool {
        matches!(self.tracks.get(i), Some(Slot::Done(_)))
    }

    /// Whether `Growing` track `i` already has at least `frames` buffered —
    /// enough to trust a crossfade into it even before it's `Done`.
    fn growing_head_at_least(&self, i: usize, frames: usize) -> bool {
        matches!(self.tracks.get(i), Some(Slot::Growing(s)) if s.len() / 2 >= frames)
    }

    fn track_len(&self, i: usize) -> usize {
        match self.tracks.get(i) {
            Some(Slot::Growing(s) | Slot::Done(s)) => s.len() / 2,
            _ => 0,
        }
    }

    fn frame(&self, i: usize, f: usize) -> (f32, f32) {
        match self.tracks.get(i) {
            Some(Slot::Growing(t) | Slot::Done(t)) if f * 2 + 1 < t.len() => {
                (t[f * 2], t[f * 2 + 1])
            }
            _ => (0.0, 0.0),
        }
    }

    fn crossfade_frames(&self) -> usize {
        let secs = f32::from_bits(self.crossfade.load(Ordering::Relaxed)).max(0.0);
        (secs * self.device_rate as f32).round() as usize
    }

    /// Pull any newly-arrived [`DecodeEvent`]s from the worker into the
    /// window, applying each to the read-side [`Slot`] state machine.
    fn drain(&mut self) {
        if let Some(rx) = &self.rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    DecodeEvent::Meta { idx, meta, capacity_frames } => {
                        if idx >= self.count {
                            continue;
                        }
                        self.metas[idx] = meta;
                        // A capacity hint arriving before any PCM: reserve it
                        // once up front so the buffer that's about to grow
                        // via `Chunk`s (below) doesn't reallocate+copy its
                        // way there piecemeal on this thread — the audio
                        // thread. Only applies to a slot with nothing in it
                        // yet (`Empty`, or a `Growing(empty)` freshly made by
                        // a `Reset`); a slot that already has PCM has already
                        // paid for whatever allocation it's using.
                        if let Some(hint) = capacity_frames {
                            match &mut self.tracks[idx] {
                                slot @ Slot::Empty => {
                                    *slot = Slot::Growing(Vec::with_capacity(hint));
                                }
                                Slot::Growing(buf) if buf.is_empty() => buf.reserve_exact(hint),
                                _ => {}
                            }
                        }
                        // Apply eagerly, even before the track is playable,
                        // so an early UI announce (title/art) can land the
                        // moment tags are known rather than waiting for the
                        // whole file — the current index is the only one the
                        // UI is showing right now.
                        //
                        // ...except mid-fade (`xf_len > 0`): the crossfade's
                        // own start already announced the *incoming* track
                        // (see `signal_index(self.index + 1)` in `read`), so
                        // re-signalling the outgoing one here would wrong-foot
                        // the UI back to it until the next real change.
                        if idx == self.index && self.xf_len == 0 {
                            self.signal_track();
                        }
                    }
                    DecodeEvent::Chunk { idx, samples } => {
                        if idx >= self.count {
                            continue;
                        }
                        match &mut self.tracks[idx] {
                            slot @ Slot::Empty => *slot = Slot::Growing(samples),
                            // An empty `Growing` (fresh, or just cleared by a
                            // `Reset`) takes the incoming `Vec` by a move —
                            // no realloc, no copy — rather than extending
                            // into it; this is also what makes the
                            // whole-track republish after a `Reset` (the
                            // fallback path's single worst case) a move too.
                            //
                            // Gated on zero *capacity*, not just zero
                            // length: an empty `Growing` can also be one a
                            // `Meta` capacity hint just reserved (see the
                            // `Meta` arm above) — moving in would silently
                            // throw that reservation away and replace it
                            // with `samples`' own (much smaller) capacity,
                            // reintroducing exactly the reallocate-as-you-go
                            // growth the hint exists to avoid. Extending
                            // into a reserved-but-empty buffer is itself
                            // already allocation-free, so falling through to
                            // the general `extend` arm below is correct and
                            // just as cheap.
                            Slot::Growing(buf) if buf.is_empty() && buf.capacity() == 0 => {
                                *buf = samples;
                            }
                            Slot::Growing(buf) => buf.extend(samples),
                            Slot::Done(_) => {
                                // The worker already said `Done` for this
                                // index — a further `Chunk` means it kept
                                // decoding past its own end-of-track signal,
                                // which is a bug in the worker, not something
                                // the read side should ever see live.
                                debug_assert!(
                                    false,
                                    "Chunk for idx {idx} after Done — worker protocol bug"
                                );
                            }
                        }
                    }
                    DecodeEvent::Done { idx } => {
                        if idx >= self.count {
                            continue;
                        }
                        let prior = std::mem::replace(&mut self.tracks[idx], Slot::Empty);
                        self.tracks[idx] = match prior {
                            Slot::Growing(s) | Slot::Done(s) => Slot::Done(s),
                            Slot::Empty => Slot::Done(Vec::new()),
                        };
                    }
                    DecodeEvent::Failed { idx } => {
                        if idx < self.count {
                            self.tracks[idx] = Slot::Done(Vec::new());
                        }
                    }
                    DecodeEvent::Reset { idx } => {
                        if idx < self.count {
                            self.tracks[idx] = Slot::Growing(Vec::new());
                        }
                        // A reset touching either side of an in-progress
                        // crossfade invalidates the ramp: `xf_len` was latched
                        // against a current-track length, or an incoming
                        // track's buffered head, that this reset just erased.
                        // Left alone, the next frame would resume blending
                        // against stale positions in tracks that no longer
                        // hold what they held a moment ago. Kill the ramp —
                        // the ordinary underrun/deferred-fade paths take over
                        // from here, exactly as if the fade had never started.
                        if self.xf_len > 0 && (idx == self.index || idx == self.index + 1) {
                            self.xf_len = 0;
                            self.xf_cursor = 0;
                        }
                    }
                }
            }
        }
    }

    /// Ask the worker to keep one track decoded ahead of the play head, and free
    /// everything already played so memory stays bounded to ~2 tracks.
    fn advance_window(&mut self) {
        if let Some(tx) = &self.want_tx {
            let _ = tx.send((self.index + 1).min(self.count.saturating_sub(1)));
        }
        for i in 0..self.index {
            if let Some(slot) = self.tracks.get_mut(i) {
                *slot = Slot::Empty;
            }
        }
    }

    fn signal_track(&self) {
        self.signal_index(self.index);
    }

    /// Announce a specific track as now-playing. Split out so a crossfade can
    /// announce the *incoming* track the moment it becomes audible, not the
    /// current one — see the streamed sibling's rationale in `queue.rs`.
    fn signal_index(&self, i: usize) {
        if let Some(idx) = &self.current_index {
            idx.store(i, Ordering::Release);
        }
        if let Some(sink) = &self.meta_sink {
            if let Some(meta) = self.metas.get(i) {
                sink.set(meta.clone());
            }
        }
    }
}

impl Drop for StreamQueueSource {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl AudioSource for StreamQueueSource {
    fn start(&mut self, _format: StreamFormat) -> Result<(), AudioError> {
        self.drain();
        self.signal_track();
        Ok(())
    }

    fn read(&mut self, out: &mut [f32], channels: usize) -> usize {
        if channels == 0 {
            return 0;
        }
        self.drain();
        let out_frames = out.len() / channels;
        let mut produced = 0;

        for f in 0..out_frames {
            let base = f * channels;
            // Past the last track: true end of stream.
            if self.index >= self.count {
                write_frame(out, base, channels, 0.0, 0.0);
                continue;
            }
            // Current track still streaming/decoding: buffer (silence, not EOF).
            if !self.ready(self.index) {
                write_frame(out, base, channels, 0.0, 0.0);
                produced += 1;
                continue;
            }

            let cur_len = self.track_len(self.index);
            let xf = self.crossfade_frames();
            // A crossfade may only start from a *finished* current track: its
            // length (`cur_len`) has to be the track's real length for
            // "within `xf` of the end" to mean anything, and a `Growing`
            // track's buffered-so-far length isn't that (see
            // `a_crossfade_defers_while_the_current_track_grows`). The next
            // track is trusted either once it's `Done`, or once it's
            // `Growing` with at least a full fade's worth already buffered —
            // otherwise the ramp could run off the end of what's arrived.
            //
            // That head also has to clear the *start gate*
            // (`start_frames`), not just the fade width: fading into a
            // `Growing` track that immediately fails `ready()` right after
            // the boundary would cut already-audible sound to silence one
            // frame later — a worse seam than the deferred fade this whole
            // mechanism exists to avoid. Trusting only a head that already
            // satisfies the start gate keeps audibility monotone across the
            // boundary.
            let next_trusted = self.index + 1 < self.count
                && (self.done(self.index + 1)
                    || self.growing_head_at_least(
                        self.index + 1,
                        xf.max(start_frames(self.device_rate)),
                    ));
            let crossfading =
                xf > 0 && self.done(self.index) && next_trusted && self.cursor + xf >= cur_len;

            let (l, r) = if crossfading {
                // Latch the ramp width on the first faded frame: whatever is
                // actually left, which is `xf` when the next track was ready on
                // time and less when it wasn't. A shorter complete fade beats a
                // full-width one truncated at the cut.
                if self.xf_len == 0 {
                    self.xf_len = xf.min(cur_len.saturating_sub(self.cursor)).max(1);
                    // Announce the incoming track at the start of the crossfade,
                    // when it becomes audible — not xf seconds later at the cut.
                    self.signal_index(self.index + 1);
                }
                let t = (self.xf_cursor as f32 / self.xf_len as f32).min(1.0);
                let (g_out, g_in) = crate::crossfade::gains(t);
                let (lc, rc) = self.frame(self.index, self.cursor);
                let (ln, rn) = self.frame(self.index + 1, self.xf_cursor);
                self.cursor += 1;
                self.xf_cursor += 1;
                if self.cursor >= cur_len {
                    self.index += 1;
                    self.cursor = self.xf_cursor;
                    self.xf_cursor = 0;
                    self.xf_len = 0;
                    // Already announced at the crossfade's start.
                    self.advance_window();
                }
                (lc * g_out + ln * g_in, rc * g_out + rn * g_in)
            } else if self.cursor < cur_len {
                let frame = self.frame(self.index, self.cursor);
                self.cursor += 1;
                frame
            } else if !self.done(self.index) {
                // Underrun: a `Growing` track's cursor has caught up to
                // everything buffered so far. Hold silence *without*
                // advancing — more `Chunk`s may still land (see `drain`),
                // and only `Done` may cross this boundary. `produced` still
                // counts this frame (below): the stream is buffering, not at
                // end-of-queue.
                (0.0, 0.0)
            } else {
                // Gapless boundary: advance to the next track.
                self.index += 1;
                self.cursor = 0;
                self.xf_len = 0;
                // A fade must never resume at a stale ramp position — this is
                // the pre-existing "slider abort" path too (e.g. a seek cut a
                // crossfade short and landed here without ever reaching the
                // crossfade branch's own reset of `xf_cursor`).
                self.xf_cursor = 0;
                self.advance_window();
                if self.ready(self.index) {
                    self.signal_track();
                    self.cursor = 1;
                    self.frame(self.index, 0)
                } else if self.index < self.count {
                    self.signal_track(); // index it, even while still buffering
                    (0.0, 0.0)
                } else {
                    (0.0, 0.0)
                }
            };

            write_frame(out, base, channels, l, r);
            if self.index < self.count {
                produced += 1;
            }
        }
        produced
    }

    fn stop(&mut self) {
        self.index = self.count;
        self.running.store(false, Ordering::Relaxed);
    }

    fn seek(&mut self, frame: usize) {
        self.cursor = frame.min(self.track_len(self.index));
        self.xf_cursor = 0;
        // Any ramp in progress belongs to where we just were.
        self.xf_len = 0;
    }

    fn position(&self) -> usize {
        // Mid-crossfade, report the incoming track's own clock — see the local
        // queue's note. `xf_cursor` flows into `cursor` at the boundary, so the
        // seek bar doesn't jump when the fade completes.
        if self.xf_len > 0 {
            self.xf_cursor.min(self.track_len(self.index + 1))
        } else {
            self.cursor.min(self.track_len(self.index))
        }
    }

    fn total_frames(&self) -> usize {
        if self.xf_len > 0 {
            self.track_len(self.index + 1)
        } else {
            self.track_len(self.index)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo(value: f32, frames: usize) -> Vec<f32> {
        vec![value; frames * 2]
    }

    /// An eager source over pre-decoded (fully `Done`) tracks (no worker), for
    /// testing the gapless/crossfade read logic. `crossfade_frames` maps
    /// directly to frames here (device_rate = 1).
    fn eager(tracks: Vec<Vec<f32>>, crossfade_frames: usize) -> StreamQueueSource {
        eager_slots(tracks.into_iter().map(Slot::Done).collect(), crossfade_frames)
    }

    /// An eager source whose slots are given explicitly (Growing/Done mixes).
    fn eager_slots(slots: Vec<Slot>, crossfade_frames: usize) -> StreamQueueSource {
        let count = slots.len();
        StreamQueueSource {
            tracks: slots,
            metas: vec![TrackMeta::default(); count],
            count,
            device_rate: 1,
            crossfade: Arc::new(AtomicU32::new((crossfade_frames as f32).to_bits())),
            index: 0,
            cursor: 0,
            xf_cursor: 0,
            xf_len: 0,
            rx: None,
            want_tx: None,
            running: Arc::new(AtomicBool::new(true)),
            meta_sink: None,
            current_index: None,
            _worker: None,
        }
    }

    /// The header that decides whether a track arrives in half a second or a
    /// hundred.
    ///
    /// googlevideo paces a plain GET to about the bitrate of what's being asked
    /// for — reasonable for a player listening in real time, ruinous for a path
    /// that must hold the whole track before it plays any of it. The same
    /// request carrying a range served the same 3.4 MB body 190× faster. Nothing
    /// in a response distinguishes the two cases, so only asking the wire can
    /// tell us the header is really going out.
    #[test]
    fn a_fetch_asks_for_the_body_as_a_range() {
        use std::io::{BufRead, BufWriter, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

        let server_seen = seen.clone();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line == "\r\n" || line == "\n" {
                    break;
                }
                server_seen.lock().unwrap().push(line.trim_end().to_string());
            }
            let body = b"not audio, but bytes";
            let mut w = BufWriter::new(stream);
            let _ = write!(
                w,
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-19/20\r\n\
                 Content-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = w.write_all(body);
            let _ = w.flush();
        });

        let client = reqwest::blocking::Client::builder().build().unwrap();
        let target = StreamTarget {
            url: format!("http://{addr}/track"),
            headers: vec![("User-Agent".into(), "hm-test".into())],
            ext: Some("m4a".into()),
        };
        // Just opening the connection is all this test is about — decoding
        // (and the body being nonsense) is someone else's test.
        let _ = open_stream(&client, &target);
        server.join().expect("server thread");

        let lines = seen.lock().unwrap().clone();
        let range = lines
            .iter()
            .find(|l| l.to_ascii_lowercase().starts_with("range:"))
            .unwrap_or_else(|| panic!("no Range header was sent; got {lines:#?}"));
        assert!(
            range.eq_ignore_ascii_case("range: bytes=0-"),
            "asked for the wrong range: {range}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.eq_ignore_ascii_case("user-agent: hm-test")),
            "the resolver's headers must still go out — the CDN checks the agent; got {lines:#?}"
        );
    }

    #[test]
    fn plays_tracks_gaplessly_then_ends() {
        let mut src = eager(vec![vec![0.1, 0.1, 0.2, 0.2], vec![0.3, 0.3, 0.4, 0.4]], 0);
        let mut out = vec![0.0f32; 10]; // 5 stereo frames
        assert_eq!(src.read(&mut out, 2), 4, "4 real frames across both tracks");
        assert_eq!(&out[0..8], &[0.1, 0.1, 0.2, 0.2, 0.3, 0.3, 0.4, 0.4]);
        assert_eq!(&out[8..10], &[0.0, 0.0], "silence past the end");

        let mut tail = vec![0.0f32; 4];
        assert_eq!(src.read(&mut tail, 2), 0, "EOF after the queue is exhausted");
    }

    /// Equal power (`cos`), not the linear `[1.0, 0.75, 0.5, 0.25]` this used to
    /// assert — a linear ramp dips −3 dB mid-fade. See `crate::crossfade`.
    #[test]
    fn crossfade_ramps_from_one_track_to_the_next() {
        let mut src = eager(vec![stereo(1.0, 4), stereo(0.0, 8)], 4);
        let mut out = vec![0.0f32; 8]; // the 4 crossfade frames
        src.read(&mut out, 2);
        let left = [out[0], out[2], out[4], out[6]];
        let want = [1.0, 0.923_88, std::f32::consts::FRAC_1_SQRT_2, 0.382_68];
        for (got, want) in left.iter().zip(want) {
            assert!((got - want).abs() < 1e-4, "crossfade ramp: {got} vs {want}");
        }
        assert!(
            left[2] > 0.7,
            "the midpoint must sit at 1/sqrt(2), not the 0.5 a linear ramp gives"
        );
    }

    /// A lookahead that lands *after* the crossfade window has opened — the
    /// normal case for a slow resolve — must still ramp all the way out.
    ///
    /// Otherwise the outgoing track is cut at whatever gain the truncated ramp
    /// reached and the incoming one jumps to full: a click, arriving exactly
    /// when the network is worst.
    /// The streamed sibling of the local queue's rule: the incoming track is
    /// announced at the crossfade's start, when it becomes audible.
    /// The seek bar rides with the now-playing title: once the crossfade begins
    /// and the incoming track is announced, the reported position and duration
    /// are *its* — not the outgoing track's clock under the next song's name.
    #[test]
    fn crossfade_reports_the_incoming_track_position_and_duration() {
        // Distinct lengths (100 vs 80) so the duration switch is unmistakable.
        let mut src = eager(vec![stereo(1.0, 100), stereo(-1.0, 80)], 40);

        // Before the window opens: the outgoing track's own timeline.
        let mut out = vec![0.0f32; 120]; // 60 frames
        src.read(&mut out, 2);
        assert_eq!(src.total_frames(), 100, "outgoing duration before the crossfade");

        // Ten frames into the crossfade: both flip to the incoming track, even
        // though the play head is still fading out of the first (index unchanged).
        let mut out = vec![0.0f32; 20]; // 10 frames
        src.read(&mut out, 2);
        assert_eq!(src.total_frames(), 80, "incoming duration during the crossfade");
        assert!(
            src.position() <= 12,
            "position is the incoming track's, near its start; got {}",
            src.position()
        );
        assert_eq!(src.index, 0, "still mid-fade, not at the boundary");
    }

    #[test]
    fn crossfade_announces_the_incoming_track_at_its_start() {
        let idx = Arc::new(AtomicUsize::new(0));
        let mut src = eager(vec![stereo(1.0, 100), stereo(-1.0, 100)], 40);
        src.current_index = Some(idx.clone());

        let mut out = vec![0.0f32; 120]; // 60 frames — up to the window
        src.read(&mut out, 2);
        assert_eq!(idx.load(Ordering::Relaxed), 0, "still the first track before the crossfade");

        let mut out = vec![0.0f32; 2]; // 1 frame into the crossfade
        src.read(&mut out, 2);
        assert_eq!(idx.load(Ordering::Relaxed), 1, "incoming track announced at the crossfade start");
        assert_eq!(src.index, 0, "still mid-fade, not at the boundary");
    }

    #[test]
    fn a_late_lookahead_still_ramps_fully_out() {
        // 100-frame tracks, 40-frame crossfade → the window opens at frame 60.
        let mut src = eager(vec![stereo(1.0, 100), stereo(-1.0, 100)], 40);
        src.tracks[1] = Slot::Empty; // still decoding — was `None` under the old `Option<Vec<f32>>`

        // Play to frame 80 — 20 frames INTO the window with nothing to fade to.
        let mut out = vec![0.0f32; 160];
        src.read(&mut out, 2);
        assert!(
            out.iter().all(|s| (*s - 1.0).abs() < 1e-6),
            "current track should play untouched while there's nothing to fade to"
        );

        // It lands now, with 20 of the current track's frames left.
        src.tracks[1] = Slot::Done(stereo(-1.0, 100)); // was `Some(...)`

        // Ramp over what's actually left: the last frame before the boundary
        // should be almost entirely the incoming track. Dividing by the full 40
        // instead only reaches the halfway point, cutting the outgoing track at
        // audible gain — the click this guards.
        let mut out = vec![0.0f32; 40];
        src.read(&mut out, 2);
        let last = out[38];
        assert!(
            last < -0.8,
            "ramp must reach the incoming track before the cut, got {last}"
        );
    }

    #[test]
    fn buffers_silence_while_a_track_is_undecoded() {
        // Track 0 not yet decoded (Empty): produce silence but NOT end-of-stream.
        let mut src = eager(vec![stereo(0.5, 4)], 0);
        src.tracks[0] = Slot::Empty; // was `None`
        let mut out = vec![0.0f32; 6];
        let produced = src.read(&mut out, 2);
        assert_eq!(produced, 3, "still 'producing' (buffering), not EOF");
        assert!(out.iter().all(|&s| s == 0.0), "all silence while buffering");
    }

    #[test]
    fn skips_a_failed_track() {
        // Track 0 decoded-but-empty (failed) → skip straight to track 1.
        let mut src = eager(vec![Vec::new(), vec![0.7, 0.7]], 0);
        let mut out = vec![0.0f32; 6];
        src.read(&mut out, 2);
        assert_eq!(&out[0..2], &[0.7, 0.7], "first real audio comes from track 1");
    }

    #[test]
    fn retry_recovers_a_transient_failure() {
        // A flaky fetch (e.g. a stale connection after a pause) fails twice then
        // succeeds — the ladder must keep trying, not give up early.
        //
        // Edited for task 4: `load_with_retry` no longer hands back decoded
        // samples (a winning attempt publishes `Meta`/`Chunk`/`Done` to the
        // read side itself — see `LoadAttempt`) or takes an `index` (nothing
        // left to tag with one), so the closure now returns `Published`
        // instead of `Ready(samples, meta)`, and the assertion is on the new
        // `LadderOutcome` rather than a returned track tuple. The ladder
        // mechanics under test — keep trying a transient failure — are
        // unchanged.
        let running = AtomicBool::new(true);
        let mut calls = 0u32;
        let outcome = load_with_retry(&running, Duration::ZERO, |_| {
            calls += 1;
            if calls < 3 {
                LoadAttempt::Retry
            } else {
                LoadAttempt::Published
            }
        });
        assert!(matches!(outcome, LadderOutcome::Published), "track recovered after retries");
        assert_eq!(calls, 3, "kept trying until it succeeded");
    }

    #[test]
    fn permanent_failure_skips_without_retrying() {
        // A 404-class failure shouldn't burn the retry budget — skip immediately.
        // Edited for task 4: see `retry_recovers_a_transient_failure` — same
        // `LoadAttempt`/`LadderOutcome` shape, same ladder behavior under test.
        let running = AtomicBool::new(true);
        let mut calls = 0u32;
        let outcome = load_with_retry(&running, Duration::ZERO, |_| {
            calls += 1;
            LoadAttempt::Skip
        });
        assert!(
            matches!(outcome, LadderOutcome::GaveUp),
            "permanent failure → gave up; the caller sends the terminal Failed"
        );
        assert_eq!(calls, 1, "no retries for a permanent failure");
    }

    #[test]
    fn gives_up_after_max_attempts() {
        // Endlessly transient → bounded retries, then gives up (so the queue
        // moves on instead of buffering forever).
        // Edited for task 4: see `retry_recovers_a_transient_failure`.
        let running = AtomicBool::new(true);
        let mut calls = 0u32;
        let outcome = load_with_retry(&running, Duration::ZERO, |_| {
            calls += 1;
            LoadAttempt::Retry
        });
        assert!(matches!(outcome, LadderOutcome::GaveUp), "exhausted retries → gave up");
        assert_eq!(calls, MAX_ATTEMPTS, "stopped at the attempt cap");
    }

    #[test]
    fn stop_request_aborts_retrying() {
        // If the source is dropped/stopped mid-retry, bail out promptly.
        // Edited for task 4: see `retry_recovers_a_transient_failure`.
        let running = AtomicBool::new(false);
        let mut calls = 0u32;
        let outcome = load_with_retry(&running, Duration::ZERO, |_| {
            calls += 1;
            LoadAttempt::Retry
        });
        assert!(matches!(outcome, LadderOutcome::GaveUp));
        assert_eq!(calls, 0, "no attempts once stopped");
    }

    #[test]
    fn spooled_body_deletes_its_temp_file_on_drop() {
        let (spool, mut file) = SpooledBody::create(Some("mp3")).expect("create spool");
        std::io::Write::write_all(&mut file, b"not really audio").unwrap();
        drop(file);
        let path = spool.path.clone();
        assert!(path.exists(), "spool file exists while the guard lives");
        assert_eq!(
            path.extension().and_then(|e| e.to_str()),
            Some("mp3"),
            "extension hint carried onto the spool file"
        );
        drop(spool);
        assert!(!path.exists(), "spool file removed when the guard drops");
    }

    #[test]
    fn spool_sanitizes_a_hostile_extension_hint() {
        // The ext hint comes from a resolver (cloud metadata) — path separators
        // or other junk must not escape the temp dir or break the filename.
        let (spool, _file) = SpooledBody::create(Some("../../etc/passwd")).expect("create spool");
        let name = spool.path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("hm-stream-"), "stays a spool file: {name}");
        assert!(!name.contains('/') && !name.contains(".."), "sanitized: {name}");
        assert_eq!(spool.path.parent(), Some(std::env::temp_dir().as_path()));
    }

    // --- Slot state machine (fast-load phase 3) -----------------------------

    #[test]
    fn a_growing_track_below_the_start_gate_buffers_silence() {
        // 2 frames buffered, but device_rate = 4 puts the start gate at 4
        // frames — below it, so this must not be allowed to start playing yet.
        let mut src = eager_slots(vec![Slot::Growing(stereo(0.5, 2))], 0);
        src.device_rate = 4;
        let mut out = vec![9.0f32; 6]; // 3 stereo frames, poisoned to catch stray writes
        let produced = src.read(&mut out, 2);
        assert_eq!(produced, 3, "still 'producing' (buffering), not EOF");
        assert!(out.iter().all(|&s| s == 0.0), "silence while below the start gate");
    }

    #[test]
    fn a_growing_track_past_the_start_gate_plays() {
        // 5 frames buffered, device_rate = 4 → the start gate is 4 frames, so
        // this is clear to start playing even though it's still Growing.
        let mut src = eager_slots(vec![Slot::Growing(stereo(0.6, 5))], 0);
        src.device_rate = 4;
        let mut out = vec![0.0f32; 8]; // 4 stereo frames
        let produced = src.read(&mut out, 2);
        assert_eq!(produced, 4);
        assert!(
            out.chunks(2).all(|f| f == [0.6, 0.6]),
            "real samples once past the start gate: {out:?}"
        );
    }

    #[test]
    fn an_underrun_on_a_growing_track_stalls_without_advancing() {
        let mut src = eager_slots(
            vec![Slot::Growing(stereo(0.4, 2)), Slot::Done(stereo(0.9, 2))],
            0,
        );
        let mut out = vec![0.0f32; 8]; // 4 frames: 2 real, then 2 past the buffered tail
        let produced = src.read(&mut out, 2);
        assert_eq!(produced, 4, "still counted as buffering, not EOF");
        assert_eq!(&out[0..4], &[0.4, 0.4, 0.4, 0.4], "the buffered frames play");
        assert_eq!(&out[4..8], &[0.0, 0.0, 0.0, 0.0], "silence once the buffer runs out");
        assert_eq!(src.index, 0, "no advance on an underrun — track 0 might still grow");

        // More PCM lands for the still-Growing track.
        if let Slot::Growing(buf) = &mut src.tracks[0] {
            buf.extend(stereo(0.4, 2));
        } else {
            panic!("track 0 should still be Growing");
        }
        let mut out2 = vec![0.0f32; 4]; // 2 frames
        let produced2 = src.read(&mut out2, 2);
        assert_eq!(produced2, 2);
        assert_eq!(&out2[0..4], &[0.4, 0.4, 0.4, 0.4], "resumes from the cursor, not from 0");
        assert_eq!(src.index, 0, "still track 0 — no boundary while it's not Done");
    }

    #[test]
    fn a_boundary_advances_only_when_done() {
        let mut src = eager_slots(
            vec![Slot::Growing(stereo(0.4, 2)), Slot::Done(stereo(0.9, 2))],
            0,
        );
        // Same shape as the underrun test: read the 2 buffered frames, hit the
        // stall, confirm it, then flip track 0 to Done and confirm the
        // boundary now fires exactly like `eager()`'s tracks always do.
        let mut out = vec![0.0f32; 8];
        src.read(&mut out, 2);
        assert_eq!(src.index, 0, "underrun stalls first, same as the sibling test");

        match std::mem::replace(&mut src.tracks[0], Slot::Empty) {
            Slot::Growing(buf) => src.tracks[0] = Slot::Done(buf),
            other => panic!("expected Growing, got {other:?}"),
        }

        let mut out2 = vec![0.0f32; 4]; // 2 frames — now crosses onto track 1
        let produced2 = src.read(&mut out2, 2);
        assert_eq!(produced2, 2);
        assert_eq!(&out2[0..4], &[0.9, 0.9, 0.9, 0.9], "advanced onto track 1 at the boundary");
        assert_eq!(src.index, 1);
    }

    #[test]
    fn a_crossfade_defers_while_the_current_track_grows() {
        let mut src = eager_slots(
            vec![Slot::Growing(stereo(1.0, 4)), Slot::Done(stereo(-1.0, 8))],
            4, // crossfade width
        );
        // The cursor catches up to everything buffered so far (4 frames):
        // with the current track still Growing, the crossfade must NOT
        // start even though the next track is fully Done and the window
        // (cursor + xf >= cur_len) is already open — this is the underrun
        // rule, not a fade.
        let mut out = vec![0.0f32; 12]; // 4 real frames + 2 stalled
        let produced = src.read(&mut out, 2);
        assert_eq!(produced, 6, "still buffering — silence-stall, not EOF");
        assert!(
            out[0..8].chunks(2).all(|f| f == [1.0, 1.0]),
            "the 4 buffered frames play plain, no blend: {out:?}"
        );
        assert_eq!(&out[8..12], &[0.0, 0.0, 0.0, 0.0], "no fade frames — silence-stall instead");
        assert_eq!(src.index, 0, "no advance while Growing, fade window open or not");

        // The current track finishes decoding — Done now, with 4 more frames
        // to actually ramp over.
        src.tracks[0] = Slot::Done({
            let mut v = stereo(1.0, 4);
            v.extend(stereo(1.0, 4));
            v
        });
        let mut out2 = vec![0.0f32; 4]; // 2 of the 4 fade frames
        src.read(&mut out2, 2);
        let left = [out2[0], out2[2]];
        assert!((left[0] - 1.0).abs() < 1e-4, "fade starts at full current gain: {left:?}");
        assert!(left[1] < left[0], "already ramping toward the incoming track: {left:?}");
        assert_eq!(src.index, 0, "still mid-fade, not yet at the boundary");
    }

    #[test]
    fn a_crossfade_into_a_growing_next_with_enough_head_ramps() {
        // Same shape as `crossfade_ramps_from_one_track_to_the_next`, but the
        // next track is still `Growing` — it just already has enough head
        // (>= the crossfade width) for the fade to be trusted.
        let mut src = eager_slots(
            vec![Slot::Done(stereo(1.0, 4)), Slot::Growing(stereo(0.0, 8))],
            4,
        );
        let mut out = vec![0.0f32; 8]; // the 4 crossfade frames
        src.read(&mut out, 2);
        let left = [out[0], out[2], out[4], out[6]];
        let want = [1.0, 0.923_88, std::f32::consts::FRAC_1_SQRT_2, 0.382_68];
        for (got, want) in left.iter().zip(want) {
            assert!((got - want).abs() < 1e-4, "crossfade ramp: {got} vs {want}");
        }
    }

    #[test]
    fn a_crossfade_waits_for_a_growing_next_below_the_fade_width() {
        // Mirrors `a_late_lookahead_still_ramps_fully_out`, but the next
        // track isn't undecided (`Empty`) — it's `Growing` with fewer frames
        // than the crossfade width, which must be treated the same: not
        // enough to trust yet, so the current track keeps playing untouched.
        let mut src = eager_slots(
            vec![Slot::Done(stereo(1.0, 100)), Slot::Growing(stereo(-1.0, 20))],
            40,
        );

        // Play to frame 80 — 20 frames INTO the window with only 20 buffered
        // on the next track (< the 40-frame width).
        let mut out = vec![0.0f32; 160];
        src.read(&mut out, 2);
        assert!(
            out.iter().all(|s| (*s - 1.0).abs() < 1e-6),
            "current track should play untouched — the next track isn't trusted yet"
        );

        // The rest of it lands, giving it plenty of head now.
        src.tracks[1] = Slot::Growing(stereo(-1.0, 100));

        // Ramp over what's actually left: the last frame before the boundary
        // should be almost entirely the incoming track.
        let mut out = vec![0.0f32; 40]; // 20 frames left before the boundary
        src.read(&mut out, 2);
        let last = out[38];
        assert!(
            last < -0.8,
            "ramp must reach the incoming track before the cut, got {last}"
        );
    }

    #[test]
    fn reset_keeps_the_cursor_and_stalls_until_redecoded() {
        let mut src = eager_slots(vec![Slot::Growing(stereo(0.3, 6))], 0);
        let mut out = vec![0.0f32; 6]; // 3 frames
        src.read(&mut out, 2);
        assert_eq!(&out[0..6], &[0.3, 0.3, 0.3, 0.3, 0.3, 0.3]);
        assert_eq!(src.cursor, 3);

        // What `drain` does for a `Reset` event: the buffer is emptied but
        // the cursor is kept, so redecoding resumes where playback already
        // is rather than restarting from the top.
        src.tracks[0] = Slot::Growing(Vec::new());
        let mut out2 = vec![0.0f32; 4]; // 2 frames
        let produced = src.read(&mut out2, 2);
        assert_eq!(produced, 2, "still buffering, not EOF");
        assert_eq!(&out2[0..4], &[0.0, 0.0, 0.0, 0.0], "silence at the same cursor");
        assert_eq!(src.cursor, 3, "cursor untouched by the reset");

        // Re-decoded, past the cursor.
        src.tracks[0] = Slot::Growing(stereo(0.7, 5));
        let mut out3 = vec![0.0f32; 4]; // 2 frames
        src.read(&mut out3, 2);
        assert_eq!(&out3[0..4], &[0.7, 0.7, 0.7, 0.7], "resumes at cursor 3, not from 0");
    }

    #[test]
    fn a_failed_track_still_skips() {
        // Pin against `skips_a_failed_track`, but built through `Slot`
        // directly: `Done(empty)` (what a `Failed` event produces) skips at
        // cursor 0, instantly, straight onto the next track.
        let mut src = eager_slots(vec![Slot::Done(Vec::new()), Slot::Done(stereo(0.7, 2))], 0);
        let mut out = vec![0.0f32; 6];
        src.read(&mut out, 2);
        assert_eq!(&out[0..2], &[0.7, 0.7], "first real audio comes from track 1");
    }

    #[test]
    fn drain_applies_the_event_protocol() {
        let (tx, rx) = mpsc::channel::<DecodeEvent>();
        let mut src = StreamQueueSource {
            tracks: vec![Slot::Empty, Slot::Empty],
            metas: vec![TrackMeta::default(); 2],
            count: 2,
            device_rate: 1,
            crossfade: Arc::new(AtomicU32::new(0.0f32.to_bits())),
            index: 0,
            cursor: 0,
            xf_cursor: 0,
            xf_len: 0,
            rx: Some(rx),
            want_tx: None,
            running: Arc::new(AtomicBool::new(true)),
            meta_sink: None,
            current_index: None,
            _worker: None,
        };

        let meta = TrackMeta {
            title: Some("applied".into()),
            ..Default::default()
        };
        tx.send(DecodeEvent::Meta { idx: 0, meta, capacity_frames: None }).unwrap();
        tx.send(DecodeEvent::Chunk { idx: 0, samples: stereo(0.1, 2) }).unwrap();
        tx.send(DecodeEvent::Chunk { idx: 0, samples: stereo(0.1, 3) }).unwrap();
        tx.send(DecodeEvent::Done { idx: 0 }).unwrap();
        tx.send(DecodeEvent::Failed { idx: 1 }).unwrap();

        src.drain();

        match &src.tracks[0] {
            Slot::Done(samples) => assert_eq!(samples.len(), (2 + 3) * 2, "chunks concatenated"),
            other => panic!("expected Done, got {other:?}"),
        }
        assert_eq!(src.metas[0].title.as_deref(), Some("applied"), "meta applied");
        match &src.tracks[1] {
            Slot::Done(samples) => assert!(samples.is_empty(), "Failed -> Done(empty)"),
            other => panic!("expected Done(empty), got {other:?}"),
        }

        tx.send(DecodeEvent::Reset { idx: 0 }).unwrap();
        src.drain();
        match &src.tracks[0] {
            Slot::Growing(samples) => assert!(samples.is_empty(), "Reset -> Growing(empty)"),
            other => panic!("expected Growing(empty), got {other:?}"),
        }
    }

    /// F1(b): a `Meta` carrying a capacity hint reserves the buffer exactly
    /// once — every `Chunk` that follows, as long as the running total stays
    /// under the hint, must find the buffer's capacity unchanged (no
    /// reallocation) rather than creeping up piecemeal via `Vec::extend`'s
    /// own amortized growth.
    #[test]
    fn meta_capacity_hint_reserves_once_and_holds_steady_across_chunks() {
        let (tx, rx) = mpsc::channel::<DecodeEvent>();
        let mut src = StreamQueueSource {
            tracks: vec![Slot::Empty],
            metas: vec![TrackMeta::default()],
            count: 1,
            device_rate: 1,
            crossfade: Arc::new(AtomicU32::new(0.0f32.to_bits())),
            index: 0,
            cursor: 0,
            xf_cursor: 0,
            xf_len: 0,
            rx: Some(rx),
            want_tx: None,
            running: Arc::new(AtomicBool::new(true)),
            meta_sink: None,
            current_index: None,
            _worker: None,
        };

        let hint = 10_000usize;
        tx.send(DecodeEvent::Meta {
            idx: 0,
            meta: TrackMeta::default(),
            capacity_frames: Some(hint),
        })
        .unwrap();
        src.drain();

        let reserved = match &src.tracks[0] {
            Slot::Growing(buf) => {
                assert!(buf.capacity() >= hint, "the hint must be reserved up front");
                buf.capacity()
            }
            other => panic!("expected Growing after a Meta with a hint, got {other:?}"),
        };

        // Several chunks, well under `hint` in total: the capacity must
        // never move — no chunk after the first should trigger a realloc.
        for _ in 0..5 {
            tx.send(DecodeEvent::Chunk { idx: 0, samples: stereo(0.1, 100) }).unwrap();
            src.drain();
            match &src.tracks[0] {
                Slot::Growing(buf) => assert_eq!(
                    buf.capacity(),
                    reserved,
                    "capacity must stay stable across appends within the reserved hint"
                ),
                other => panic!("expected Growing, got {other:?}"),
            }
        }
    }

    #[test]
    fn meta_reaches_the_sink_early_but_not_mid_fade() {
        let (tx, rx) = mpsc::channel::<DecodeEvent>();
        let (sink, meta_handle) = MetaSink::for_test();
        let current_index = Arc::new(AtomicUsize::new(0));
        let mut src = StreamQueueSource {
            tracks: vec![Slot::Empty],
            metas: vec![TrackMeta::default()],
            count: 1,
            device_rate: 1,
            crossfade: Arc::new(AtomicU32::new(0.0f32.to_bits())),
            index: 0,
            cursor: 0,
            xf_cursor: 0,
            xf_len: 0,
            rx: Some(rx),
            want_tx: None,
            running: Arc::new(AtomicBool::new(true)),
            meta_sink: Some(sink),
            current_index: Some(current_index.clone()),
            _worker: None,
        };

        // `Meta` for the current index, arriving before the track is even
        // playable (the slot is still `Empty`) — the early announce this
        // whole path exists for.
        let first = TrackMeta {
            title: Some("first".into()),
            ..Default::default()
        };
        tx.send(DecodeEvent::Meta { idx: 0, meta: first, capacity_frames: None }).unwrap();
        src.drain();
        assert_eq!(
            meta_handle.load().title.as_deref(),
            Some("first"),
            "the sink saw the early announce"
        );
        assert_eq!(current_index.load(Ordering::Relaxed), 0);

        // Now simulate being mid-crossfade: the incoming track was already
        // announced at the fade's start (`signal_index` in `read`), so a
        // fresh `Meta` for the *outgoing* current index must not re-signal
        // and wrong-foot the UI back to it.
        src.xf_len = 4;
        let second = TrackMeta {
            title: Some("second".into()),
            ..Default::default()
        };
        tx.send(DecodeEvent::Meta { idx: 0, meta: second, capacity_frames: None }).unwrap();
        src.drain();
        assert_eq!(
            src.metas[0].title.as_deref(),
            Some("second"),
            "internal bookkeeping still updates"
        );
        assert_eq!(
            meta_handle.load().title.as_deref(),
            Some("first"),
            "but the sink is NOT re-signalled mid-fade"
        );
    }

    // --- Streaming worker pieces (fast-load phase 3, task 4) ----------------

    /// A `Read` that hands back bytes a few at a time (so partial reads are
    /// exercised, not one big slurp) and then fails once exhausted — standing
    /// in for a connection dropped mid-body.
    struct FlakyReader {
        remaining: Vec<u8>,
        fail_when_empty: bool,
    }

    impl std::io::Read for FlakyReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.remaining.is_empty() {
                return if self.fail_when_empty {
                    Err(std::io::Error::other("connection reset"))
                } else {
                    Ok(0)
                };
            }
            let n = buf.len().min(self.remaining.len()).min(3);
            for (i, b) in self.remaining.drain(0..n).enumerate() {
                buf[i] = b;
            }
            Ok(n)
        }
    }

    #[test]
    fn tee_reader_passes_bytes_through_and_mirrors_them_to_the_file() {
        let data = b"hello streaming world".to_vec();
        let mut reader = FlakyReader { remaining: data.clone(), fail_when_empty: false };
        let path = std::env::temp_dir().join("hm_tee_reader_test_passthrough.bin");
        let mut file = std::fs::File::create(&path).unwrap();
        let mut seen = 0u64;
        let mut out = Vec::new();
        {
            let mut tee = TeeReader { inner: &mut reader, file: &mut file, seen: &mut seen };
            std::io::Read::read_to_end(&mut tee, &mut out).unwrap();
        }
        assert_eq!(out, data, "every byte read passes through to the caller");
        assert_eq!(seen, data.len() as u64, "the counter matches bytes read");
        drop(file);
        let mirrored = std::fs::read(&path).unwrap();
        assert_eq!(mirrored, data, "every byte read also landed in the file");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tee_reader_propagates_the_inner_error_after_partial_reads() {
        let data = b"partial-then-broken".to_vec();
        let mut reader = FlakyReader { remaining: data.clone(), fail_when_empty: true };
        let path = std::env::temp_dir().join("hm_tee_reader_test_partial_err.bin");
        let mut file = std::fs::File::create(&path).unwrap();
        let mut seen = 0u64;
        let mut out = Vec::new();
        let result = {
            let mut tee = TeeReader { inner: &mut reader, file: &mut file, seen: &mut seen };
            std::io::Read::read_to_end(&mut tee, &mut out)
        };
        assert!(result.is_err(), "the inner error must propagate, not be swallowed");
        assert_eq!(out, data, "bytes read before the failure still reached the caller");
        assert_eq!(seen, data.len() as u64, "the counter reflects everything read before the error");
        drop(file);
        let mirrored = std::fs::read(&path).unwrap();
        assert_eq!(mirrored, data, "bytes read before the failure still landed in the file");
        let _ = std::fs::remove_file(&path);
    }

    // These three pin the raw `(bytes_seen, declared) -> StreamFailure`
    // predicate itself — a pure function, agnostic to *when* `bytes_seen`
    // was measured. The worker (`stream_decode_attempt`) only ever calls it
    // with the POST-drain count (see `drain_then_classify` and the tests
    // below), never the count at the moment of the original decode/probe
    // error — that sequencing is what makes an incomplete-body decode error
    // retryable-with-a-fallback-shot rather than an immediate `Retry`.
    #[test]
    fn after_stream_failure_classifies_a_complete_body_as_decode_spool() {
        assert!(matches!(after_stream_failure(20, Some(20)), StreamFailure::DecodeSpool));
        assert!(
            matches!(after_stream_failure(25, Some(20)), StreamFailure::DecodeSpool),
            "more than declared is still complete"
        );
    }

    #[test]
    fn after_stream_failure_classifies_a_short_body_as_retry() {
        assert!(matches!(after_stream_failure(10, Some(20)), StreamFailure::Retry));
    }

    #[test]
    fn after_stream_failure_with_no_declared_length_is_lenient() {
        // Can't prove truncation without a declared length — same lenient
        // posture as the pre-streaming `is_none_or` check in `fetch_once`.
        assert!(matches!(after_stream_failure(3, None), StreamFailure::DecodeSpool));
    }

    #[test]
    fn finish_or_retry_matches_the_same_predicate_both_directions() {
        assert!(finish_or_retry(20, Some(20)), "a complete body may finish");
        assert!(!finish_or_retry(10, Some(20)), "a short body must not finish as a short Done");
        assert!(finish_or_retry(3, None), "no declared length: same lenient posture as DecodeSpool");
    }

    #[test]
    fn drain_then_classify_uses_the_post_drain_byte_count_not_the_pre_drain_one() {
        // Pins the ordering the coordinator's fix depends on: classification
        // happens on whatever `drain` reports *after* running, never on a
        // byte count captured before it. A decode/probe error on an
        // incomplete body (e.g. a non-faststart mp4/m4a that the streaming
        // demuxer gave up on after only a few KB) must get its drain-and-see
        // shot at the whole-file fallback, not an immediate `Retry`.
        let result = drain_then_classify(Some(20), || 20 /* drain completed the body */);
        assert!(
            matches!(result, StreamFailure::DecodeSpool),
            "the drain finished the body -> whole-file fallback, not Retry"
        );
    }

    #[test]
    fn drain_then_classify_still_retries_when_the_drain_itself_comes_up_short() {
        // The drain is given every chance (bounded only by the same
        // `FETCH_TIMEOUT` already on the response) — if it *still* comes up
        // short, that's a genuine transport failure, not a container that
        // merely needed more bytes.
        let result = drain_then_classify(Some(20), || 10 /* drain itself came up short */);
        assert!(matches!(result, StreamFailure::Retry), "still short after draining -> Retry");
    }

    #[test]
    fn drain_then_classify_is_lenient_with_no_declared_length() {
        let result = drain_then_classify(None, || 3);
        assert!(
            matches!(result, StreamFailure::DecodeSpool),
            "no declared length: same lenient posture as the underlying predicate"
        );
    }

    // --- `finish_or_drain_then_retry` (coordinator round 3: the riff/WAV
    // trailing-chunk fix) — the decode-`Ok`-side mirror of
    // `drain_then_classify`, pinned the same way: an injected drain result,
    // no real I/O, just the sequencing/short-circuit contract.

    #[test]
    fn finish_or_drain_then_retry_is_true_without_draining_when_already_complete() {
        // The common case: the tee's own count already proves completeness,
        // so `drain` must never even be called — a `panic!` in the closure
        // would fail this test if the short-circuit were lost.
        assert!(finish_or_drain_then_retry(20, Some(20), || panic!("must not drain")));
    }

    #[test]
    fn finish_or_drain_then_retry_trusts_a_drain_that_completes_the_body() {
        // The riff/WAV/AIFF case: a clean `Ok` decode with `seen < declared`
        // isn't necessarily a truncation — it might be trailing chunks the
        // demuxer never reads (a `LIST`/id3 tag after `data`). Draining the
        // rest and finding the body complete after all must trust it, not
        // retry a perfectly good file.
        assert!(finish_or_drain_then_retry(10, Some(20), || 20));
    }

    #[test]
    fn finish_or_drain_then_retry_still_retries_when_the_drain_stays_short() {
        assert!(!finish_or_drain_then_retry(10, Some(20), || 15));
    }

    #[test]
    fn finish_or_drain_then_retry_is_lenient_with_no_declared_length() {
        assert!(finish_or_drain_then_retry(3, None, || panic!("must not drain")));
    }

    // --- `stream_decode_attempt` event-sequence tests (coordinator round 3)
    //
    // Drives the real function end to end over a local TCP harness (same
    // pattern as `a_fetch_asks_for_the_body_as_a_range`), using decode.rs's
    // `tiny_wav` for a real, probeable body, and inspects the exact sequence
    // of `DecodeEvent`s it publishes.
    //
    // Test (c) from the brief (a fallback case exercised through this same
    // harness — a streaming-probe failure whose complete body IS decodable
    // whole) is NOT implemented: WAV/PCM's demuxer has no seek dependency
    // for a well-formed file (unlike a non-faststart mp4/m4a), so there is no
    // cheap way to make the STREAMING probe fail while the identical bytes,
    // decoded whole from the spool, succeed — both paths run the exact same
    // header parse. Constructing that would need a real seek-dependent
    // container fixture (e.g. a hand-rolled mp4 with `moov` at the tail),
    // which is a nontrivial fixture to hand-build reliably here. The
    // fallback's *classification* logic is still fully pinned at the pure-fn
    // level (`after_stream_failure`/`drain_then_classify`'s `DecodeSpool`
    // tests above), and its wiring is identical code to (and shares every
    // line with) the already-tested truncation-recovery path below — see the
    // task report for the full note.

    /// Spins up a one-shot local HTTP server that accepts exactly one
    /// connection, ignores whatever request it receives, and replies with a
    /// 200 whose `Content-Length` is `declared_len` — independent of how
    /// many bytes of `body` are actually written — before closing the
    /// connection. `declared_len == body.len()` serves a normal complete
    /// body; `declared_len > body.len()` simulates a connection that closes
    /// before delivering everything it said it would.
    fn serve_body(
        body: Vec<u8>,
        declared_len: usize,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        use std::io::{BufRead, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line == "\r\n" || line == "\n" {
                    break;
                }
            }
            let mut w = std::io::BufWriter::new(stream);
            let _ = write!(w, "HTTP/1.1 200 OK\r\nContent-Length: {declared_len}\r\n\r\n");
            let _ = w.write_all(&body);
            let _ = w.flush();
            // Dropping `w` (and the underlying `TcpStream`) here closes the
            // connection — for a short `body` that's exactly the "server
            // declared more than it sent" truncation this harness exists
            // to simulate.
        });
        (addr, handle)
    }

    /// Runs one `stream_decode_attempt` over `serve_body`'s server and
    /// returns every `DecodeEvent` it published, in order, plus the
    /// attempt's own `LoadAttempt` classification.
    fn run_stream_decode_attempt(
        body: Vec<u8>,
        declared_len: usize,
    ) -> (Vec<DecodeEvent>, LoadAttempt) {
        let (addr, server) = serve_body(body, declared_len);
        let client = reqwest::blocking::Client::builder().build().unwrap();
        let target = StreamTarget {
            url: format!("http://{addr}/track"),
            headers: vec![],
            ext: Some("wav".into()),
        };
        let (tx, rx) = mpsc::channel::<DecodeEvent>();
        let running = AtomicBool::new(true);

        let attempt = match open_stream(&client, &target) {
            Opened::Body { resp, declared } => {
                stream_decode_attempt(0, resp, declared, target.ext.as_deref(), 44_100, &tx, &running)
            }
            _ => panic!("expected Opened::Body from a 200 response"),
        };
        server.join().expect("server thread");
        drop(tx);
        (rx.try_iter().collect(), attempt)
    }

    #[test]
    fn a_clean_complete_stream_publishes_exactly_meta_chunk_star_done() {
        let wav = crate::decode::tests::tiny_wav(10_000, 44_100);
        let (events, attempt) = run_stream_decode_attempt(wav.clone(), wav.len());
        assert!(
            matches!(attempt, LoadAttempt::Published),
            "a clean complete stream must report Published, not retry or skip"
        );

        let mut saw_meta = false;
        let mut chunk_count = 0u32;
        let mut done_count = 0u32;
        let mut reset_count = 0u32;
        for e in &events {
            match e {
                DecodeEvent::Meta { .. } => {
                    assert_eq!(chunk_count, 0, "Meta must arrive before any Chunk");
                    assert_eq!(done_count, 0, "Meta must arrive before Done");
                    saw_meta = true;
                }
                DecodeEvent::Chunk { .. } => {
                    assert!(saw_meta, "Chunk must arrive after Meta");
                    assert_eq!(done_count, 0, "no Chunk after Done");
                    chunk_count += 1;
                }
                DecodeEvent::Done { .. } => {
                    assert_eq!(done_count, 0, "exactly one Done");
                    done_count += 1;
                }
                DecodeEvent::Reset { .. } => reset_count += 1,
                DecodeEvent::Failed { .. } => panic!("a clean complete body must not fail"),
            }
        }
        assert!(saw_meta, "Meta must be published");
        assert!(chunk_count >= 1, "at least one Chunk must be published");
        assert_eq!(done_count, 1, "exactly one terminal Done, no more, no less");
        assert_eq!(reset_count, 0, "a clean stream never needs a Reset");
    }

    #[test]
    fn a_body_shorter_than_its_declared_length_never_publishes_a_done() {
        // A WAV whose header/`data`-chunk-size claims far more than what the
        // server actually sends before closing — the truncation the
        // no-short-`Done` guarantee exists for. Big enough (100k frames) to
        // cross `CHUNK_FRAMES` (48_000) at least once via the main decode
        // loop before the connection dies, so `chunks_sent` is deterministic
        // rather than depending on whether a final partial flush happens to
        // run.
        let full = crate::decode::tests::tiny_wav(100_000, 44_100);
        let declared_len = full.len();
        let truncated = full[..(44 + 60_000 * 4)].to_vec(); // header + 60k real frames
        let (events, attempt) = run_stream_decode_attempt(truncated, declared_len);
        assert!(
            matches!(attempt, LoadAttempt::Retry),
            "a truncated body must report Retry, not Published"
        );

        let chunk_count = events.iter().filter(|e| matches!(e, DecodeEvent::Chunk { .. })).count();
        let reset_count = events.iter().filter(|e| matches!(e, DecodeEvent::Reset { .. })).count();
        assert!(
            !events.iter().any(|e| matches!(e, DecodeEvent::Done { .. })),
            "a truncated body must NEVER publish a terminal Done"
        );
        assert!(chunk_count >= 1, "60k buffered frames must cross CHUNK_FRAMES at least once");
        assert_eq!(
            reset_count, 1,
            "Reset must appear exactly once, since Chunks were published for this attempt"
        );
    }
}
