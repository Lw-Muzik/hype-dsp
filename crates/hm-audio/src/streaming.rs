//! HTTP audio streaming source (internet radio, cloud files, phone tracks).
//!
//! A dedicated thread does a blocking HTTP GET, decodes the stream with
//! symphonia (over a non-seekable [`ReadOnlySource`]), resamples to the device
//! rate, and pushes interleaved-stereo samples into a lock-free SPSC ring. The
//! audio callback pulls from the ring via [`AudioSource::read`], emitting
//! silence while buffering.
//!
//! ## Duration & seeking
//!
//! When the stream's total length is known — from the container (symphonia
//! `num_frames`) or an external hint (e.g. a phone track's `durationMs`) — the
//! source reports a duration and becomes **seekable**: a seek re-opens the
//! connection with an HTTP `Range` header at the byte offset matching the
//! target time (accurate for CBR, approximate for VBR). A stream with no known
//! length stays **live** (open-ended radio): it never reports EOF on underflow
//! and cannot be scrubbed.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use rtrb::{Producer, RingBuffer};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;

use crate::decode::resample_stereo;
use crate::error::AudioError;
use crate::AudioSource;

/// Lock-free transport state shared between the source (audio thread) and the
/// background decode thread.
struct StreamShared {
    running: AtomicBool,
    /// Frames (at device rate) played so far / sought to.
    position_frames: AtomicU64,
    /// Total length in device-rate frames, or 0 if unknown (live).
    total_frames: AtomicU64,
    /// Full HTTP body length in bytes, or 0 if the server didn't report it.
    content_bytes: AtomicU64,
    /// Decode reached the natural end of the stream and won't produce more.
    finished: AtomicBool,
    /// Pending seek target in device-rate frames, or -1 for none.
    seek_target: AtomicI64,
    /// While set, `read` drains and discards the ring (post-seek flush).
    flushing: AtomicBool,
    /// Most recent download throughput estimate, bytes/sec (EWMA, 0 until measured).
    download_bps: AtomicU64,
    /// Count of mid-track rebuffering events (transitions into buffering after initial fill).
    rebuffer_count: AtomicU32,
}

impl StreamShared {
    fn duration_secs(&self, device_rate: u32) -> f64 {
        if device_rate == 0 {
            return 0.0;
        }
        self.total_frames.load(Ordering::Relaxed) as f64 / device_rate as f64
    }
}

/// Buffering/ring sizing for a stream, derived from the network mode. Larger on
/// constrained links so a slow download builds a cushion instead of stuttering.
#[derive(Clone, Copy, Debug)]
pub struct StreamTuning {
    /// Frames the ring must hold before (re)starting playback.
    pub prebuffer_frames: usize,
    /// Ring capacity in frames (stereo → 2× this many samples).
    pub ring_frames: usize,
}

impl StreamTuning {
    /// Default tuning for a device rate, picking bigger buffers in Data Saver.
    pub fn for_network(device_rate: u32, data_saver: bool) -> Self {
        let rate = device_rate.max(8_000) as usize;
        let prebuffer_secs = if data_saver { 6 } else { 2 };
        let ring_secs = if data_saver { 45 } else { 30 };
        Self {
            prebuffer_frames: rate * prebuffer_secs,
            ring_frames: rate * ring_secs,
        }
    }
}

/// Whether playback should stay gated (emit buffering silence) this block.
/// Once playing (`buffering == false`) the gate is open; an empty ring is
/// handled inside the read loop, which re-arms `buffering` on a full drain.
fn should_buffer(
    available_frames: usize,
    prebuffer_frames: usize,
    finished: bool,
    buffering: bool,
) -> bool {
    buffering && !(finished || available_frames >= prebuffer_frames)
}

/// An HTTP audio stream rendered as an [`AudioSource`].
pub struct RadioStreamSource {
    consumer: rtrb::Consumer<f32>,
    shared: Arc<StreamShared>,
    device_rate: u32,
    /// Target cushion (frames) before (re)starting playback.
    prebuffer_frames: usize,
    /// True while we're holding for the cushion to fill (start or after a drain).
    buffering: bool,
    _thread: JoinHandle<()>,
}

impl RadioStreamSource {
    /// Start streaming `url`, producing stereo at `device_rate`.
    pub fn new(url: String, device_rate: u32) -> Self {
        Self::with_headers(
            url,
            Vec::new(),
            device_rate,
            None,
            None,
            StreamTuning::for_network(device_rate, false),
        )
    }

    /// Start streaming `url` with extra HTTP request headers (e.g. an
    /// `Authorization: Bearer …` for a Google Drive `alt=media` URL). When a
    /// [`MetaSink`] is provided, tags + cover art are published to it once the
    /// stream has been probed. `duration_hint` (seconds) is used as the track
    /// length when the container doesn't carry one (e.g. a raw stream whose
    /// duration the phone already knows), which makes the stream seekable.
    pub fn with_headers(
        url: String,
        headers: Vec<(String, String)>,
        device_rate: u32,
        meta_sink: Option<crate::engine::MetaSink>,
        duration_hint: Option<f64>,
        tuning: StreamTuning,
    ) -> Self {
        let capacity = tuning.ring_frames * 2; // stereo
        let (producer, consumer) = RingBuffer::<f32>::new(capacity);
        let shared = Arc::new(StreamShared {
            running: AtomicBool::new(true),
            position_frames: AtomicU64::new(0),
            total_frames: AtomicU64::new(0),
            content_bytes: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            seek_target: AtomicI64::new(-1),
            flushing: AtomicBool::new(false),
            download_bps: AtomicU64::new(0),
            rebuffer_count: AtomicU32::new(0),
        });

        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("hm-stream-decode".into())
                .spawn(move || {
                    stream_worker(
                        &url,
                        &headers,
                        device_rate,
                        producer,
                        shared,
                        meta_sink,
                        duration_hint,
                    )
                })
                .expect("failed to spawn stream decode thread")
        };

        Self {
            consumer,
            shared,
            device_rate,
            prebuffer_frames: tuning.prebuffer_frames,
            buffering: true,
            _thread: thread,
        }
    }
}

impl RadioStreamSource {
    /// Most recent download throughput estimate, bytes/sec (0 until measured).
    pub fn download_bps(&self) -> u64 {
        self.shared.download_bps.load(Ordering::Relaxed)
    }
    /// Count of mid-track rebuffering events so far this stream.
    pub fn rebuffer_count(&self) -> u32 {
        self.shared.rebuffer_count.load(Ordering::Relaxed)
    }
    /// Whether playback is currently held waiting for the buffer to fill.
    pub fn is_buffering(&self) -> bool {
        self.buffering
    }
}

impl Drop for RadioStreamSource {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::Relaxed);
    }
}

/// Wraps a reader and tallies bytes successfully read, so the worker knows how
/// far it got before a connection dropped (for `Range`-based resume).
struct CountingReader<R> {
    inner: R,
    count: Arc<AtomicU64>,
}

impl<R: std::io::Read> std::io::Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Opens the resource at an absolute byte offset, yielding a fresh body. The
/// transport is behind a trait so the resumable reader below can be exercised
/// without a network.
trait RangeOpener: Send + Sync {
    /// Open at `offset`. `Err` marks a transport failure the caller may retry.
    fn open_at(&self, offset: u64) -> std::io::Result<Box<dyn std::io::Read + Send + Sync>>;
}

/// [`RangeOpener`] over HTTP: re-issues the original GET — same URL, same
/// headers — with a `Range` at the requested offset.
struct HttpRanges {
    client: reqwest::blocking::Client,
    url: String,
    headers: Vec<(String, String)>,
}

impl RangeOpener for HttpRanges {
    fn open_at(&self, offset: u64) -> std::io::Result<Box<dyn std::io::Read + Send + Sync>> {
        match open(&self.client, &self.url, &self.headers, offset) {
            Some(response) => Ok(Box::new(response)),
            // `open` also rejects a server that answered a ranged request with
            // the whole body, which would splice byte 0 onto the middle of the
            // stream — worse than no resume at all.
            None => Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!("no ranged body at byte {offset}"),
            )),
        }
    }
}

/// A body that outlives its connection: a read that fails — or that stops short
/// of the resource's length — is served by silently re-opening at the byte the
/// last one stopped on, so the consumer sees one unbroken stream.
///
/// This has to sit **beneath** symphonia. A paused track stops draining the
/// ring; the ring fills, the body stops being read, and the server hangs up on
/// the idle socket — long before the URL itself expires, which for a signed CDN
/// link is hours away. Re-opening above the demuxer cannot recover from that: a
/// container's header exists only at byte 0, so probing a body that starts
/// mid-file fails outright (an MP4 has no `ftyp`/`moov` there). Splicing the
/// bytes back together instead means the demuxer and the decoder never learn
/// there was a tear, and neither is ever reset.
///
/// Reads run on the decode thread, never the audio callback, so blocking here to
/// re-open is safe.
struct ResumableBody {
    ranges: Arc<dyn RangeOpener>,
    /// The open body, or `None` while there is none to read from.
    body: Option<Box<dyn std::io::Read + Send + Sync>>,
    /// Absolute offset of the next byte to deliver.
    offset: u64,
    /// Full resource length in bytes, or 0 when unknown. Without a length a
    /// short body is indistinguishable from a complete one and there is no
    /// offset worth resuming at, so an open-ended stream (radio) is never
    /// resumed — it just ends, as it always has.
    content_bytes: u64,
    /// Consecutive re-opens since the last byte was delivered.
    retries: u32,
    /// Cancellation: a re-open in flight gives up when the stream is stopped.
    shared: Arc<StreamShared>,
}

impl ResumableBody {
    /// Wrap an already-open `body` whose first byte is at absolute `start_byte`.
    /// `content_bytes` is the full resource length, or 0 when unknown.
    fn new(
        ranges: Arc<dyn RangeOpener>,
        body: Box<dyn std::io::Read + Send + Sync>,
        start_byte: u64,
        content_bytes: u64,
        shared: Arc<StreamShared>,
    ) -> Self {
        Self {
            ranges,
            body: Some(body),
            offset: start_byte,
            content_bytes,
            retries: 0,
            shared,
        }
    }

    /// Whether there is nothing left to deliver — the only circumstance in which
    /// a body going quiet is the real end of the resource rather than a drop.
    fn complete(&self) -> bool {
        self.content_bytes == 0 || self.offset >= self.content_bytes
    }

    /// Re-open at the current offset so the next read carries on exactly where
    /// the dead one stopped. Bounded on the same ladder as the initial connect:
    /// a server that keeps hanging up, or a URL that really has expired, must
    /// surface a fault instead of spinning.
    fn reopen(&mut self) -> std::io::Result<()> {
        self.body = None;
        loop {
            if !self.shared.running.load(Ordering::Relaxed) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "stream stopped",
                ));
            }
            if self.retries >= MAX_STALLS {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    format!("dropped at byte {} and could not be resumed", self.offset),
                ));
            }
            self.retries += 1;
            std::thread::sleep(retry_backoff(self.retries));
            if let Ok(body) = self.ranges.open_at(self.offset) {
                self.body = Some(body);
                return Ok(());
            }
        }
    }
}

impl std::io::Read for ResumableBody {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            let outcome = self.body.as_mut().map(|body| body.read(buf));
            match outcome {
                Some(Ok(n)) if n > 0 => {
                    self.offset += n as u64;
                    self.retries = 0;
                    return Ok(n);
                }
                // Ran out exactly where the resource does (or where an
                // open-ended one chose to) → a real end of stream.
                Some(Ok(_)) if self.complete() => return Ok(0),
                // An open-ended stream has nothing to splice onto.
                Some(Err(e)) if self.content_bytes == 0 => return Err(e),
                // A short body or a broken socket: the connection died, the
                // resource did not. Ask for the rest of it.
                _ => self.reopen()?,
            }
        }
    }
}

impl AudioSource for RadioStreamSource {
    fn start(&mut self, _format: crate::StreamFormat) -> Result<(), AudioError> {
        Ok(())
    }

    fn read(&mut self, out: &mut [f32], channels: usize) -> usize {
        if channels == 0 {
            return 0;
        }
        let frames = out.len() / channels;

        // Post-seek flush: drop everything buffered and emit silence. Counted as
        // "produced" so the renderer treats it as buffering, not end-of-stream.
        if self.shared.flushing.load(Ordering::Relaxed) {
            while self.consumer.pop().is_ok() {}
            for s in out.iter_mut() {
                *s = 0.0;
            }
            return frames;
        }

        let finished = self.shared.finished.load(Ordering::Relaxed);
        let available = self.consumer.slots() / 2;
        if should_buffer(available, self.prebuffer_frames, finished, self.buffering) {
            for s in out.iter_mut() {
                *s = 0.0;
            }
            return frames; // buffering: silence, counted as produced (not EOF)
        }
        self.buffering = false;

        let mut produced = 0;
        let mut popped = 0u64;
        for f in 0..frames {
            let base = f * channels;
            if self.consumer.slots() >= 2 {
                let l = self.consumer.pop().unwrap_or(0.0);
                let r = self.consumer.pop().unwrap_or(0.0);
                popped += 1;
                produced += 1;
                if channels == 1 {
                    out[base] = 0.5 * (l + r);
                } else {
                    out[base] = l;
                    out[base + 1] = r;
                    for ch in out.iter_mut().take(base + channels).skip(base + 2) {
                        *ch = 0.0;
                    }
                }
            } else {
                for ch in out.iter_mut().take(base + channels).skip(base) {
                    *ch = 0.0;
                }
                // Still decoding: emit silence but keep playback alive. Once the
                // decode thread is done and the ring is drained, stop producing
                // so the renderer can advance to the next track.
                if !finished {
                    produced += 1;
                }
            }
        }
        self.shared
            .position_frames
            .fetch_add(popped, Ordering::Relaxed);
        if !finished && self.consumer.slots() < 2 && !self.buffering {
            self.buffering = true;
            self.shared.rebuffer_count.fetch_add(1, Ordering::Relaxed);
        }
        produced
    }

    fn stop(&mut self) {
        self.shared.running.store(false, Ordering::Relaxed);
    }

    fn seek(&mut self, frame: usize) {
        if !self.seekable() {
            return;
        }
        // Snap the reported position immediately and start discarding the ring;
        // the decode thread re-opens the connection at the matching byte offset.
        self.shared.flushing.store(true, Ordering::Relaxed);
        self.shared
            .position_frames
            .store(frame as u64, Ordering::Relaxed);
        self.shared.finished.store(false, Ordering::Relaxed);
        self.shared
            .seek_target
            .store(frame as i64, Ordering::Relaxed);
        while self.consumer.pop().is_ok() {}
    }

    fn position(&self) -> usize {
        self.shared.position_frames.load(Ordering::Relaxed) as usize
    }

    fn total_frames(&self) -> usize {
        self.shared.total_frames.load(Ordering::Relaxed) as usize
    }

    fn seekable(&self) -> bool {
        self.shared.content_bytes.load(Ordering::Relaxed) > 0
            && self.shared.total_frames.load(Ordering::Relaxed) > 0
    }

    fn is_live(&self) -> bool {
        // Open-ended (radio) until we learn a duration; finite once we do.
        self.shared.total_frames.load(Ordering::Relaxed) == 0
            && self.device_rate > 0
    }

    fn buffering(&self) -> bool {
        self.is_buffering()
    }
    fn download_bps(&self) -> u64 {
        self.download_bps()
    }
    fn rebuffer_count(&self) -> u32 {
        self.rebuffer_count()
    }
}

/// Why a single connection's decode loop stopped.
#[derive(Debug, PartialEq, Eq)]
enum Stop {
    /// Streaming was cancelled (source dropped / stopped).
    Cancelled,
    /// The stream was fully consumed.
    Eof,
    /// The connection carried nothing decodable: it couldn't be probed, or it
    /// held no usable audio track.
    Undecodable,
    /// The transport or the decoder faulted. The track is *not* over.
    Fault,
    /// A seek to `device-rate frame` was requested.
    Seek(u64),
}

/// Classify a symphonia error. Symphonia reports the natural end of a stream as
/// an unexpected-EOF io error, and that is the *only* error that means the track
/// is over; everything else is a fault. Blurring the two hands the renderer a
/// clean end-of-track whenever the network so much as hiccups, and it advances
/// to the next song mid-play.
fn stop_for(shared: &StreamShared, err: SymError) -> Stop {
    match err {
        SymError::IoError(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Stop::Eof,
        // Tearing the stream down surfaces as whatever error the reader was in
        // the middle of; the stop flag is the authority on why.
        _ if !shared.running.load(Ordering::Relaxed) => Stop::Cancelled,
        _ => Stop::Fault,
    }
}

/// How many no-progress reconnects in a row we tolerate before giving up on a
/// track (so a server that keeps closing, or a container we can't re-probe
/// mid-file, ends the track instead of hot-looping).
const MAX_STALLS: u32 = 3;

/// Backoff before the `attempt`-th consecutive retry, shared by the initial
/// connect and a mid-stream resume — a flaky/2G link takes a moment to come back.
fn retry_backoff(attempt: u32) -> Duration {
    Duration::from_millis(400 * attempt as u64)
}

/// What a stopped connection contributed: how far the resume offset may advance,
/// and whether it got anywhere at all.
///
/// A connection that yielded nothing decodable contributed nothing, however many
/// bytes it read — those reads were swallowed by the probe's look-ahead, so
/// folding them into the offset would skip audio on the next `Range`, and,
/// worse, would read as progress and keep [`MAX_STALLS`] from ever engaging:
/// the worker would re-probe its way to the end of the body a look-ahead at a
/// time and call that a finished track.
fn connection_progress(stop: &Stop, conn_bytes: u64) -> (u64, bool) {
    match stop {
        Stop::Undecodable => (0, false),
        _ => (conn_bytes, conn_bytes > 0),
    }
}

/// What to do after a connection's decode loop stops.
#[derive(Debug, PartialEq, Eq)]
enum ResumeDecision {
    /// The track is genuinely done (reached the end, unknown length, or we gave
    /// up after too many stalls) — report EOF.
    Finish,
    /// The connection dropped early — re-open with `Range: bytes=offset-`.
    Resume { offset: u64, stalls: u32 },
}

/// Decide whether a stopped connection is a real end or a recoverable drop.
/// `consumed` = total bytes read so far; `progressed` = did the just-ended
/// connection read new bytes; `stalls` = prior consecutive no-progress count.
fn resume_decision(content_bytes: u64, consumed: u64, progressed: bool, stalls: u32) -> ResumeDecision {
    // Unknown length (live/radio) or the whole body consumed → genuine end.
    if content_bytes == 0 || consumed >= content_bytes {
        return ResumeDecision::Finish;
    }
    let stalls = if progressed { 0 } else { stalls + 1 };
    if stalls > MAX_STALLS {
        ResumeDecision::Finish
    } else {
        ResumeDecision::Resume { offset: consumed, stalls }
    }
}

/// One blocking client for every stream in the process.
///
/// A fresh client per stream re-did the TLS handshake for every track — on
/// the same googlevideo host, back to back, ~100-300ms a time. reqwest pools
/// connections per client, so sharing one is what makes consecutive tracks
/// reuse the socket. Config matches both call sites' *client-level* settings:
/// `connect_timeout` only. It deliberately carries no overall `.timeout()` —
/// `stream_worker` below reads from an open connection for as long as a track
/// plays (minutes), which a whole-request timeout would cut off mid-song.
/// A caller that needs a total-request deadline for a one-shot download
/// (`stream_queue.rs`'s lookahead fetch) adds it per-request via
/// `RequestBuilder::timeout`, not here.
pub(crate) fn shared_client() -> &'static reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(12))
            .build()
            .expect("default TLS config must build")
    })
}

/// Owns the connect → decode → re-open lifecycle, re-opening with a byte-range
/// request on each seek and idling at EOF until a seek or cancellation.
fn stream_worker(
    url: &str,
    headers: &[(String, String)],
    device_rate: u32,
    mut producer: Producer<f32>,
    shared: Arc<StreamShared>,
    meta_sink: Option<crate::engine::MetaSink>,
    duration_hint: Option<f64>,
) {
    let client = shared_client().clone();

    // Extension hint helps symphonia pick a demuxer when content sniffing alone
    // is ambiguous (e.g. raw AAC/MP3).
    let ext = url
        .split(['?', '#'])
        .next()
        .and_then(|p| p.rsplit('.').next())
        .map(str::to_ascii_lowercase)
        .filter(|e| matches!(e.as_str(), "mp3" | "aac" | "ogg" | "flac" | "m4a" | "mp4" | "wav"));

    // Every body — the first and each transparent resume beneath the decoder —
    // comes from here, so a re-open replays the caller's headers verbatim.
    let ranges: Arc<dyn RangeOpener> = Arc::new(HttpRanges {
        client: client.clone(),
        url: url.to_string(),
        headers: headers.to_vec(),
    });

    let mut start_byte = 0u64;
    let mut meta_published = false;
    let mut stalls = 0u32;
    let conn_bytes = Arc::new(AtomicU64::new(0));
    let mut connect_fails = 0u32;
    let mut meter_start = std::time::Instant::now();
    let mut meter_bytes = 0u64;

    loop {
        if !shared.running.load(Ordering::Relaxed) {
            return;
        }
        conn_bytes.store(0, Ordering::Relaxed);

        let Some(response) = open(&client, url, headers, start_byte) else {
            // Couldn't (re)open. Retry a few times (2G connect is slow/flaky),
            // then fall back to the start once, then give up.
            connect_fails += 1;
            if connect_fails <= MAX_STALLS {
                std::thread::sleep(retry_backoff(connect_fails));
                continue;
            }
            if start_byte > 0 {
                start_byte = 0;
                connect_fails = 0;
                continue;
            }
            // Out of options. The stream must be marked finished on the way out:
            // an unfinished stream with a dead worker has `read` reporting
            // progress forever — silence that never ends and never errors.
            shared.finished.store(true, Ordering::Relaxed);
            return;
        };
        connect_fails = 0;
        record_content_length(&shared, &response, start_byte);

        let sink = if meta_published { None } else { meta_sink.clone() };
        let stop = decode_connection(
            ResumableBody::new(
                ranges.clone(),
                Box::new(response),
                start_byte,
                shared.content_bytes.load(Ordering::Relaxed),
                shared.clone(),
            ),
            conn_bytes.clone(),
            device_rate,
            &mut producer,
            &shared,
            sink,
            duration_hint,
            start_byte,
            ext.as_deref(),
            &mut meta_published,
        );

        // The counter sits above the resumable body, so it tallies bytes handed
        // to the decoder across any resumes the body did on its own — i.e. how
        // far this attempt actually got.
        let (advanced, progressed) = connection_progress(&stop, conn_bytes.load(Ordering::Relaxed));
        let consumed = start_byte + advanced;

        // Update EWMA download throughput ~once per second.
        meter_bytes += conn_bytes.load(Ordering::Relaxed);
        let elapsed = meter_start.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            let bps = (meter_bytes as f64 / elapsed) as u64;
            let prev = shared.download_bps.load(Ordering::Relaxed);
            // EWMA (3:1) to smooth bursts.
            let smoothed = if prev == 0 { bps } else { (prev * 3 + bps) / 4 };
            shared.download_bps.store(smoothed, Ordering::Relaxed);
            meter_start = std::time::Instant::now();
            meter_bytes = 0;
        }

        match stop {
            Stop::Cancelled => return,
            Stop::Seek(target) => {
                start_byte = byte_offset(&shared, device_rate, target);
                shared.finished.store(false, Ordering::Relaxed);
                stalls = 0;
            }
            // The end of the body, a fault the body couldn't splice over, or a
            // connection that got nowhere: only the byte count can tell a
            // finished track from one owed another attempt.
            Stop::Eof | Stop::Fault | Stop::Undecodable => {
                let total = shared.content_bytes.load(Ordering::Relaxed);
                match resume_decision(total, consumed, progressed, stalls) {
                    ResumeDecision::Resume { offset, stalls: s } => {
                        // Connection dropped mid-track — resume, don't end it.
                        start_byte = offset;
                        stalls = s;
                        shared.finished.store(false, Ordering::Relaxed);
                        std::thread::sleep(Duration::from_millis(300));
                        continue;
                    }
                    ResumeDecision::Finish => {
                        shared.finished.store(true, Ordering::Relaxed);
                        // Idle: a finished-but-seekable stream can still be scrubbed.
                        loop {
                            if !shared.running.load(Ordering::Relaxed) {
                                return;
                            }
                            let target = shared.seek_target.swap(-1, Ordering::Relaxed);
                            if target >= 0 {
                                start_byte = byte_offset(&shared, device_rate, target as u64);
                                shared.finished.store(false, Ordering::Relaxed);
                                stalls = 0;
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(25));
                        }
                    }
                }
            }
        }
    }
}

/// Leading bytes streamed for the fast metadata path — covers the front-loaded
/// ID3v2 / FLAC / MP4 headers (incl. a normal embedded cover) of most files.
const META_PREFIX_SMALL: u64 = 2 * 1024 * 1024;
/// Larger window buffered for the fallback probe — big enough to hold a whole
/// typical song so tags at the END (ID3v1, MP4 `moov`-at-end) or past a large
/// embedded cover are read too. Bounds the download for huge lossless files.
const META_PREFIX_LARGE: u64 = 24 * 1024 * 1024;

/// Read title/artist/album + embedded cover from a remote audio file without
/// pulling the whole track — the desktop counterpart to the mobile app's
/// `MediaMetadataRetriever`-over-URL cloud metadata. `ext` hints the container.
///
/// A small leading prefix resolves most files cheaply; when that finds nothing
/// (tags past the prefix, or only at the end of the file — ID3v1 / `moov`-at-end,
/// common in re-encoded MP3s), it falls back to buffering a larger chunk and
/// probing it **seekably**, so end-of-file metadata is read just like a local
/// file. Returns `None` only if the file can't be fetched/probed at all.
pub fn fetch_stream_metadata(
    url: &str,
    headers: &[(String, String)],
    ext: Option<&str>,
) -> Option<hm_core::TrackMeta> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(6))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()?;

    // Fast path: stream just the leading bytes (no buffering).
    if let Some(meta) = probe_stream(&client, url, headers, ext, META_PREFIX_SMALL) {
        if meta_useful(&meta) {
            return Some(meta);
        }
    }
    // Fallback: buffer a larger chunk and probe it seekably so end-of-file tags
    // are picked up. Cached by the caller, so this only runs once per file.
    probe_buffered(&client, url, headers, ext, META_PREFIX_LARGE)
}

fn meta_useful(m: &hm_core::TrackMeta) -> bool {
    m.title.is_some() || m.artist.is_some() || m.album.is_some() || m.cover.is_some()
}

fn meta_request(
    client: &reqwest::blocking::Client,
    url: &str,
    headers: &[(String, String)],
    max_bytes: u64,
) -> Option<reqwest::blocking::Response> {
    let mut req = client
        .get(url)
        .header("Range", format!("bytes=0-{}", max_bytes - 1));
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    match req.send() {
        Ok(r) if r.status().is_success() => Some(r),
        _ => None,
    }
}

fn meta_from_format(mss: MediaSourceStream, ext: Option<&str>) -> Option<hm_core::TrackMeta> {
    let mut hint = Hint::new();
    if let Some(ext) = ext {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .ok()?;
    Some(crate::meta::extract_metadata(&mut *format))
}

/// Stream the leading `max_bytes` (forward-only, unbuffered) and probe.
fn probe_stream(
    client: &reqwest::blocking::Client,
    url: &str,
    headers: &[(String, String)],
    ext: Option<&str>,
    max_bytes: u64,
) -> Option<hm_core::TrackMeta> {
    use std::io::Read;
    let resp = meta_request(client, url, headers, max_bytes)?;
    let capped = resp.take(max_bytes); // cap in case the server ignored the Range
    let mss = MediaSourceStream::new(Box::new(ReadOnlySource::new(capped)), Default::default());
    meta_from_format(mss, ext)
}

/// Download up to `max_bytes` into memory and probe it as a **seekable** source,
/// so symphonia can reach metadata anywhere in the buffer. Falls back to a
/// manual ID3v1 tail parse for old/re-encoded MP3s that carry only that.
fn probe_buffered(
    client: &reqwest::blocking::Client,
    url: &str,
    headers: &[(String, String)],
    ext: Option<&str>,
    max_bytes: u64,
) -> Option<hm_core::TrackMeta> {
    use std::io::Read;
    let resp = meta_request(client, url, headers, max_bytes)?;
    // Cap the read at `max_bytes` in case the server ignored the Range header,
    // so a non-compliant host can't stream a whole large file into RAM here
    // (the sibling `probe_stream` caps the same way).
    let mut bytes = Vec::new();
    resp.take(max_bytes).read_to_end(&mut bytes).ok()?;
    // Parse the ID3v1 tail first (cheap, borrows) before the buffer is moved into
    // the seekable source for the richer symphonia probe.
    let id3v1 = parse_id3v1(&bytes);
    let cursor = std::io::Cursor::new(bytes);
    let from_symphonia = meta_from_format(
        MediaSourceStream::new(Box::new(cursor), Default::default()),
        ext,
    );
    // Prefer symphonia (covers, Unicode, all containers); fall back to ID3v1.
    match from_symphonia {
        Some(m) if meta_useful(&m) => Some(m),
        other => id3v1.or(other),
    }
}

/// Parse a 128-byte ID3v1 tail tag (`TAG` + title/artist/album, latin1) if
/// present. No cover (ID3v1 has none) — just the text, for files that lack any
/// front tag. Returns `None` when absent or empty.
fn parse_id3v1(buf: &[u8]) -> Option<hm_core::TrackMeta> {
    if buf.len() < 128 {
        return None;
    }
    let tag = &buf[buf.len() - 128..];
    if &tag[0..3] != b"TAG" {
        return None;
    }
    let field = |raw: &[u8]| {
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let s = String::from_utf8_lossy(&raw[..end]).trim().to_string();
        (!s.is_empty()).then_some(s)
    };
    let meta = hm_core::TrackMeta {
        title: field(&tag[3..33]),
        artist: field(&tag[33..63]),
        album: field(&tag[63..93]),
        cover: None,
    };
    meta_useful(&meta).then_some(meta)
}

/// Issue the GET, always as a range request from `start_byte`.
///
/// Always ranged, even from byte 0: googlevideo paces a plain GET to about
/// the bitrate of the content — reasonable for a dumb player, ruinous for a
/// buffer trying to get ahead of the decoder — while the same request with a
/// `Range` header is served at full speed (the gapless path measured 190×;
/// see `stream_queue.rs`). Servers that ignore Range answer 200 with the
/// whole body, which at byte 0 is exactly what was asked for.
fn open(
    client: &reqwest::blocking::Client,
    url: &str,
    headers: &[(String, String)],
    start_byte: u64,
) -> Option<reqwest::blocking::Response> {
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    req = req.header("Range", format!("bytes={start_byte}-"));
    match req.send() {
        Ok(r) if start_byte > 0 => {
            // A ranged resume MUST come back as 206; a 200 means the server
            // ignored the Range and would replay the whole body from byte 0
            // (audible duplication + inflated byte count). Treat that as a
            // failed open so the bounded reconnect ladder handles it.
            (r.status() == reqwest::StatusCode::PARTIAL_CONTENT).then_some(r)
        }
        // At byte 0 both a 206 (ranged) and a 200 (Range-ignoring server —
        // internet radio, some CDNs) deliver the body from the start.
        Ok(r) if r.status().is_success() => Some(r),
        _ => None,
    }
}

/// Learn the full body length (for byte-offset seeking) from the response.
fn record_content_length(
    shared: &StreamShared,
    response: &reqwest::blocking::Response,
    start_byte: u64,
) {
    if shared.content_bytes.load(Ordering::Relaxed) > 0 {
        return; // already known
    }
    // A 206 carries `Content-Range: bytes a-b/total`; a 200 carries the full
    // `Content-Length`.
    let total = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.rsplit('/').next())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .or_else(|| {
            response
                .content_length()
                .map(|len| len + start_byte)
        });
    if let Some(total) = total.filter(|&t| t > 0) {
        shared.content_bytes.store(total, Ordering::Relaxed);
    }
}

/// Byte offset for a seek to `target` device-rate frames, mapping time → bytes
/// proportionally (exact for CBR, approximate for VBR).
fn byte_offset(shared: &StreamShared, device_rate: u32, target: u64) -> u64 {
    let duration = shared.duration_secs(device_rate);
    let content = shared.content_bytes.load(Ordering::Relaxed);
    if duration <= 0.0 || content == 0 || device_rate == 0 {
        return 0;
    }
    let secs = target as f64 / device_rate as f64;
    let frac = (secs / duration).clamp(0.0, 1.0);
    ((frac * content as f64) as u64).min(content.saturating_sub(1))
}

/// Probe + decode one body, pushing resampled stereo into the ring. Returns why
/// it stopped. On the initial open (`start_byte == 0`) it also learns the
/// duration and publishes track metadata.
///
/// The body is probed exactly once, here: [`ResumableBody`] hides a dropped
/// connection underneath, so nothing below this point ever re-probes or resets
/// the decoder mid-track.
#[allow(clippy::too_many_arguments)]
fn decode_connection(
    body: ResumableBody,
    conn_bytes: Arc<AtomicU64>,
    device_rate: u32,
    producer: &mut Producer<f32>,
    shared: &StreamShared,
    meta_sink: Option<crate::engine::MetaSink>,
    duration_hint: Option<f64>,
    start_byte: u64,
    ext: Option<&str>,
    meta_published: &mut bool,
) -> Stop {
    let counted = CountingReader { inner: body, count: conn_bytes };
    let mss = MediaSourceStream::new(Box::new(ReadOnlySource::new(counted)), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext {
        hint.with_extension(ext);
    }

    let Ok(mut format) = symphonia::default::get_probe().probe(
        &hint,
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) else {
        // A container's header lives at byte 0, so a body starting anywhere else
        // can't be probed. Whatever the probe read on the way to failing is gone
        // into its own look-ahead, so this attempt got nowhere.
        return Stop::Undecodable;
    };

    if let Some(sink) = &meta_sink {
        sink.set(crate::meta::extract_metadata(&mut *format));
        *meta_published = true;
    }

    let Some(track) = format.default_track(TrackType::Audio) else {
        return Stop::Undecodable;
    };
    let track_id = track.id;
    let Some(params) = track.codec_params.as_ref().and_then(|c| c.audio()).cloned() else {
        return Stop::Undecodable;
    };
    let stream_rate = params.sample_rate.unwrap_or(44_100);

    // Learn the duration once, on the first (full) connection.
    if start_byte == 0 && shared.total_frames.load(Ordering::Relaxed) == 0 {
        let container_secs = track
            .num_frames
            .filter(|&n| n > 0)
            .map(|n| n as f64 / stream_rate as f64);
        if let Some(secs) = container_secs.or(duration_hint).filter(|&s| s > 0.0) {
            let total = (secs * device_rate as f64).round() as u64;
            shared.total_frames.store(total, Ordering::Relaxed);
        }
    }

    let Ok(mut decoder) = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
    else {
        return Stop::Undecodable;
    };

    let mut scratch: Vec<f32> = Vec::new();
    let mut first_block = true;
    loop {
        if !shared.running.load(Ordering::Relaxed) {
            return Stop::Cancelled;
        }
        let target = shared.seek_target.swap(-1, Ordering::Relaxed);
        if target >= 0 {
            return Stop::Seek(target as u64);
        }

        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => return Stop::Eof,
            Err(e) => return stop_for(shared, e),
        };
        if packet.track_id != track_id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(a) => a,
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return stop_for(shared, e),
        };
        let ch = audio.spec().channels().count().max(1);
        scratch.clear();
        audio.copy_to_vec_interleaved::<f32>(&mut scratch);

        let stereo = to_stereo(&scratch, ch);
        let resampled = resample_stereo(&stereo, stream_rate, device_rate);

        // First fresh block after a (re)open: clear the post-seek flush so the
        // ring contents from here on are the new, sought-to audio.
        if first_block {
            first_block = false;
            shared.flushing.store(false, Ordering::Relaxed);
        }
        match push_all(producer, &resampled, shared) {
            PushResult::Ok => {}
            PushResult::Cancelled => return Stop::Cancelled,
            PushResult::Seek(t) => return Stop::Seek(t),
        }
    }
}

fn to_stereo(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels == 2 {
        return interleaved.to_vec();
    }
    let frames = interleaved.len() / channels;
    let mut out = Vec::with_capacity(frames * 2);
    for f in 0..frames {
        let base = f * channels;
        if channels == 1 {
            let m = interleaved[base];
            out.push(m);
            out.push(m);
        } else {
            out.push(interleaved[base]);
            out.push(interleaved[base + 1]);
        }
    }
    out
}

enum PushResult {
    Ok,
    Cancelled,
    Seek(u64),
}

/// Push all samples, backpressuring (sleep) when the ring is full. Bails early
/// on cancellation or a pending seek so the worker can re-open promptly.
fn push_all(producer: &mut Producer<f32>, samples: &[f32], shared: &StreamShared) -> PushResult {
    for &s in samples {
        loop {
            if !shared.running.load(Ordering::Relaxed) {
                return PushResult::Cancelled;
            }
            let target = shared.seek_target.load(Ordering::Relaxed);
            if target >= 0 {
                shared.seek_target.store(-1, Ordering::Relaxed);
                return PushResult::Seek(target as u64);
            }
            match producer.push(s) {
                Ok(()) => break,
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
    PushResult::Ok
}

#[cfg(test)]
impl RadioStreamSource {
    /// Build a source over a caller-owned ring (no network), for testing the
    /// read/prebuffer gate. `prebuffer_frames` is the cushion under test.
    fn for_test(consumer: rtrb::Consumer<f32>, prebuffer_frames: usize) -> Self {
        let shared = Arc::new(StreamShared {
            running: AtomicBool::new(true),
            position_frames: AtomicU64::new(0),
            total_frames: AtomicU64::new(0),
            content_bytes: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            seek_target: AtomicI64::new(-1),
            flushing: AtomicBool::new(false),
            download_bps: AtomicU64::new(0),
            rebuffer_count: AtomicU32::new(0),
        });
        Self {
            consumer,
            shared,
            device_rate: 1,
            prebuffer_frames,
            buffering: true,
            _thread: std::thread::spawn(|| {}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::Read;
    use std::sync::Mutex;

    /// How one body from [`FakeRanges`] behaves.
    #[derive(Clone, Copy)]
    enum Serve {
        /// Serve `bytes` from the requested offset, then die — with a socket
        /// error when `err`, otherwise by simply going quiet (a short body).
        Dies { bytes: usize, err: bool },
        /// Serve the rest of the resource.
        Whole,
        /// Refuse to open at all.
        Refused,
    }

    /// An in-memory body that dies on cue, standing in for a dropped socket.
    struct FakeBody {
        data: std::io::Cursor<Vec<u8>>,
        limit: Option<usize>,
        served: usize,
        err: bool,
    }

    impl Read for FakeBody {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let Some(limit) = self.limit else {
                return self.data.read(buf);
            };
            if self.served >= limit {
                return if self.err {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        "socket dropped",
                    ))
                } else {
                    Ok(0)
                };
            }
            let room = (limit - self.served).min(buf.len());
            let n = self.data.read(&mut buf[..room])?;
            self.served += n;
            Ok(n)
        }
    }

    /// A [`RangeOpener`] serving slices of an in-memory resource. Each open pops
    /// the next scripted behaviour, so a test spells out exactly which bodies
    /// drop and where; past the end of the script every body serves in full.
    struct FakeRanges {
        data: Vec<u8>,
        script: Mutex<VecDeque<Serve>>,
        /// Absolute offsets `open_at` was asked for, in order.
        opens: Mutex<Vec<u64>>,
    }

    impl FakeRanges {
        fn new(len: usize, script: &[Serve]) -> Arc<Self> {
            // A byte pattern with a long period, so a splice landing at the
            // wrong offset can't coincidentally match.
            let data = (0..len).map(|i| (i % 251) as u8).collect();
            Arc::new(Self {
                data,
                script: Mutex::new(script.iter().copied().collect()),
                opens: Mutex::new(Vec::new()),
            })
        }
        fn opens(&self) -> Vec<u64> {
            self.opens.lock().unwrap().clone()
        }
    }

    impl RangeOpener for FakeRanges {
        fn open_at(&self, offset: u64) -> std::io::Result<Box<dyn Read + Send + Sync>> {
            self.opens.lock().unwrap().push(offset);
            let serve = self.script.lock().unwrap().pop_front().unwrap_or(Serve::Whole);
            let (limit, err) = match serve {
                Serve::Refused => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        "refused",
                    ))
                }
                Serve::Dies { bytes, err } => (Some(bytes), err),
                Serve::Whole => (None, false),
            };
            Ok(Box::new(FakeBody {
                data: std::io::Cursor::new(self.data[offset as usize..].to_vec()),
                limit,
                served: 0,
                err,
            }))
        }
    }

    /// Open the first body from `ranges` and wrap it, as the worker does.
    fn resumable(ranges: Arc<FakeRanges>, content_bytes: u64) -> ResumableBody {
        let first = ranges.open_at(0).expect("first open");
        ResumableBody::new(ranges, first, 0, content_bytes, Arc::new(shared(0, content_bytes)))
    }

    #[test]
    fn resumable_body_splices_a_dropped_connection_invisibly() {
        let ranges = FakeRanges::new(8192, &[Serve::Dies { bytes: 3000, err: true }]);
        let mut body = resumable(ranges.clone(), 8192);
        let mut got = Vec::new();
        body.read_to_end(&mut got).expect("resumed past the drop");
        assert_eq!(got, ranges.data, "delivers exactly what an unbroken read would");
        assert_eq!(
            ranges.opens(),
            vec![0, 3000],
            "re-requests from the byte the dead connection stopped on"
        );
    }

    #[test]
    fn resumable_body_reranges_at_the_absolute_offset_after_a_premature_eof() {
        // Two short bodies in a row: the offsets must accumulate absolutely,
        // not restart per connection.
        let ranges = FakeRanges::new(8192, &[
            Serve::Dies { bytes: 1000, err: false },
            Serve::Dies { bytes: 2500, err: false },
        ]);
        let mut body = resumable(ranges.clone(), 8192);
        let mut got = Vec::new();
        body.read_to_end(&mut got).expect("resumed past both short bodies");
        assert_eq!(got, ranges.data);
        assert_eq!(ranges.opens(), vec![0, 1000, 3500]);
    }

    #[test]
    fn resumable_body_errors_rather_than_faking_an_end_when_retries_run_out() {
        let ranges = FakeRanges::new(8192, &[
            Serve::Dies { bytes: 500, err: true },
            Serve::Refused,
            Serve::Refused,
            Serve::Refused,
            Serve::Refused,
        ]);
        let mut body = resumable(ranges.clone(), 8192);
        let mut got = Vec::new();
        let err = body.read_to_end(&mut got).expect_err("must not report a clean EOF");
        assert_eq!(err.kind(), std::io::ErrorKind::ConnectionAborted);
        assert_eq!(got.len(), 500, "keeps what it did deliver");
        assert_eq!(ranges.opens().len(), 1 + MAX_STALLS as usize, "bounded retries");
    }

    #[test]
    fn resumable_body_leaves_an_open_ended_stream_alone() {
        // Radio: no content length, so a body going quiet *is* the end — there's
        // nothing to resume and no offset to resume at.
        let ranges = FakeRanges::new(8192, &[Serve::Dies { bytes: 500, err: false }]);
        let mut body = resumable(ranges.clone(), 0);
        let mut got = Vec::new();
        body.read_to_end(&mut got).expect("a short live body is just the end");
        assert_eq!(got.len(), 500);
        assert_eq!(ranges.opens(), vec![0], "never re-opens a live stream");
    }

    #[test]
    fn resumable_body_stops_when_the_stream_is_cancelled() {
        let ranges = FakeRanges::new(8192, &[Serve::Dies { bytes: 100, err: true }, Serve::Refused]);
        let state = Arc::new(shared(0, 8192));
        let first = ranges.open_at(0).unwrap();
        let mut body = ResumableBody::new(ranges.clone(), first, 0, 8192, state.clone());
        let mut buf = [0u8; 100];
        body.read_exact(&mut buf).unwrap();
        state.running.store(false, Ordering::Relaxed);
        let err = body.read(&mut buf).expect_err("cancellation is not an EOF");
        assert_eq!(err.kind(), std::io::ErrorKind::ConnectionAborted);
    }

    #[test]
    fn end_of_stream_is_an_end_but_a_broken_socket_is_a_fault() {
        let s = shared(0, 0);
        // Symphonia signals the natural end of a stream as an unexpected EOF.
        let eof = SymError::IoError(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        assert_eq!(stop_for(&s, eof), Stop::Eof);
        // A dropped connection is not the end of the song.
        let reset = SymError::IoError(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        assert_eq!(stop_for(&s, reset), Stop::Fault);
        assert_eq!(stop_for(&s, SymError::ResetRequired), Stop::Fault);
        // Once stopped, any error the reader was mid-flight on means cancelled.
        s.running.store(false, Ordering::Relaxed);
        let reset = SymError::IoError(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        assert_eq!(stop_for(&s, reset), Stop::Cancelled);
    }

    #[test]
    fn a_failed_probe_counts_as_no_progress_so_stalls_engage() {
        // The probe reads a look-ahead before failing, but those bytes are its
        // own — they must not advance the offset or look like progress.
        assert_eq!(connection_progress(&Stop::Undecodable, 64 * 1024), (0, false));
        assert_eq!(connection_progress(&Stop::Fault, 64 * 1024), (64 * 1024, true));
        assert_eq!(connection_progress(&Stop::Eof, 0), (0, false));

        // Re-probing a body that starts mid-file fails every time, so the worker
        // must give up rather than creep to the end of the body one look-ahead
        // at a time (which would read as a finished track and skip the song).
        let total = 3_449_447u64;
        let mut consumed = 1_000_000u64;
        let mut stalls = 0u32;
        let mut attempts = 0u32;
        let finished = loop {
            attempts += 1;
            assert!(attempts < 100, "probe failures must not loop forever");
            let (advanced, progressed) = connection_progress(&Stop::Undecodable, 64 * 1024);
            consumed += advanced;
            match resume_decision(total, consumed, progressed, stalls) {
                ResumeDecision::Resume { offset, stalls: s } => {
                    assert_eq!(offset, 1_000_000, "a failed probe never moves the offset");
                    stalls = s;
                }
                ResumeDecision::Finish => break true,
            }
        };
        assert!(finished);
        assert_eq!(attempts, MAX_STALLS + 1, "gives up on the ladder, not at the body's end");
    }

    #[test]
    fn counting_reader_counts_bytes_read() {
        let count = Arc::new(AtomicU64::new(0));
        let data = vec![1u8, 2, 3, 4, 5, 6, 7];
        let mut r = CountingReader { inner: std::io::Cursor::new(data), count: count.clone() };
        let mut buf = [0u8; 4];
        assert_eq!(r.read(&mut buf).unwrap(), 4);
        assert_eq!(count.load(Ordering::Relaxed), 4);
        let mut rest = Vec::new();
        r.read_to_end(&mut rest).unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 7, "counts every byte read");
    }

    /// Build a 128-byte ID3v1 tag with the given fields (padded with NULs).
    fn id3v1(title: &str, artist: &str, album: &str) -> Vec<u8> {
        let mut tag = vec![0u8; 128];
        tag[0..3].copy_from_slice(b"TAG");
        let put = |tag: &mut [u8], off: usize, s: &str| {
            let b = s.as_bytes();
            let n = b.len().min(30);
            tag[off..off + n].copy_from_slice(&b[..n]);
        };
        put(&mut tag, 3, title);
        put(&mut tag, 33, artist);
        put(&mut tag, 63, album);
        tag
    }

    #[test]
    fn parses_id3v1_tail() {
        // 64 KB of "audio" followed by an ID3v1 tag.
        let mut buf = vec![0x55u8; 64 * 1024];
        buf.extend_from_slice(&id3v1("Get Lucky", "Daft Punk", "Random Access Memories"));
        let m = parse_id3v1(&buf).expect("tag present");
        assert_eq!(m.title.as_deref(), Some("Get Lucky"));
        assert_eq!(m.artist.as_deref(), Some("Daft Punk"));
        assert_eq!(m.album.as_deref(), Some("Random Access Memories"));
        assert!(m.cover.is_none());
    }

    #[test]
    fn no_id3v1_without_tag() {
        assert!(parse_id3v1(&vec![0u8; 4096]).is_none()); // no "TAG" marker
        assert!(parse_id3v1(b"short").is_none()); // < 128 bytes
        assert!(parse_id3v1(&id3v1("", "", "")).is_none()); // empty fields
    }

    fn shared(total_frames: u64, content_bytes: u64) -> StreamShared {
        StreamShared {
            running: AtomicBool::new(true),
            position_frames: AtomicU64::new(0),
            total_frames: AtomicU64::new(total_frames),
            content_bytes: AtomicU64::new(content_bytes),
            finished: AtomicBool::new(false),
            seek_target: AtomicI64::new(-1),
            flushing: AtomicBool::new(false),
            download_bps: AtomicU64::new(0),
            rebuffer_count: AtomicU32::new(0),
        }
    }

    #[test]
    fn byte_offset_maps_time_to_bytes_proportionally() {
        let rate = 48_000u32;
        // 100 s track, 1 MB body.
        let s = shared(rate as u64 * 100, 1_000_000);
        // Halfway through ⇒ ~half the bytes.
        assert_eq!(byte_offset(&s, rate, rate as u64 * 50), 500_000);
        // Start ⇒ byte 0.
        assert_eq!(byte_offset(&s, rate, 0), 0);
        // Beyond the end clamps inside the body.
        assert_eq!(byte_offset(&s, rate, rate as u64 * 200), 999_999);
    }

    #[test]
    fn byte_offset_is_zero_without_a_known_length() {
        let rate = 48_000u32;
        // No duration, or no content length ⇒ not seekable ⇒ offset 0.
        assert_eq!(byte_offset(&shared(0, 1_000_000), rate, rate as u64 * 10), 0);
        assert_eq!(byte_offset(&shared(rate as u64 * 100, 0), rate, rate as u64 * 10), 0);
    }

    #[test]
    fn to_stereo_upmixes_mono_and_passes_stereo_through() {
        assert_eq!(to_stereo(&[0.5, 0.7], 1), vec![0.5, 0.5, 0.7, 0.7]);
        let stereo = [0.1, 0.2, 0.3, 0.4];
        assert_eq!(to_stereo(&stereo, 2), stereo.to_vec());
    }

    #[test]
    fn resume_decision_distinguishes_drop_from_end() {
        // Known length, not all consumed, made progress → resume from the offset.
        assert!(matches!(
            resume_decision(1000, 400, true, 0),
            ResumeDecision::Resume { offset: 400, stalls: 0 }
        ));
        // Reached the end → finish.
        assert!(matches!(resume_decision(1000, 1000, true, 0), ResumeDecision::Finish));
        // Unknown length (radio / no content-length) → finish (unchanged behaviour).
        assert!(matches!(resume_decision(0, 12345, false, 0), ResumeDecision::Finish));
        // A stalled reconnect (no progress) increments the counter…
        assert!(matches!(
            resume_decision(1000, 400, false, 1),
            ResumeDecision::Resume { offset: 400, stalls: 2 }
        ));
        // …until it exceeds the cap, then give up (finish so the queue advances).
        assert!(matches!(resume_decision(1000, 400, false, MAX_STALLS), ResumeDecision::Finish));
        // Progress resets the stall counter even at a high prior count.
        assert!(matches!(
            resume_decision(1000, 700, true, MAX_STALLS),
            ResumeDecision::Resume { offset: 700, stalls: 0 }
        ));
    }

    #[test]
    fn should_buffer_gates_until_prebuffer_then_releases() {
        // While buffering: hold until we have the cushion (or finished).
        assert!(should_buffer(1, 4, false, true), "below target → keep buffering");
        assert!(!should_buffer(4, 4, false, true), "met target → release");
        assert!(!should_buffer(0, 4, true, true), "finished → release even if short");
        // Once playing, the gate is open (underrun handled inside the read loop).
        assert!(!should_buffer(0, 4, false, false));
    }

    #[test]
    fn for_network_uses_larger_buffers_in_data_saver() {
        let normal = StreamTuning::for_network(48_000, false);
        let saver = StreamTuning::for_network(48_000, true);
        assert!(saver.prebuffer_frames > normal.prebuffer_frames);
        assert!(saver.ring_frames >= normal.ring_frames);
        assert!(normal.prebuffer_frames > 0 && normal.ring_frames > normal.prebuffer_frames);
    }

    #[test]
    fn read_holds_silence_until_prebuffered_then_plays() {
        // 4-frame prebuffer; push 2 → buffering (silence, produced>0, not EOF).
        let (mut prod, src_consumer) = RingBuffer::<f32>::new(64);
        for _ in 0..2 { prod.push(0.5).unwrap(); prod.push(0.5).unwrap(); }
        let mut src = RadioStreamSource::for_test(src_consumer, 4);
        let mut out = vec![0.0f32; 6]; // 3 frames
        assert_eq!(src.read(&mut out, 2), 3, "buffering counts as produced (not EOF)");
        assert!(out.iter().all(|&s| s == 0.0), "silence while buffering");
        // Top up past the target → it releases and plays real audio.
        for _ in 0..4 { prod.push(0.5).unwrap(); prod.push(0.5).unwrap(); }
        let mut out2 = vec![0.0f32; 4];
        src.read(&mut out2, 2);
        assert_eq!(out2[0], 0.5, "plays buffered audio once the cushion is met");
    }

    #[test]
    fn read_counts_rebuffer_events() {
        let (mut prod, consumer) = RingBuffer::<f32>::new(64);
        for _ in 0..6 { prod.push(0.5).unwrap(); prod.push(0.5).unwrap(); }
        let mut src = RadioStreamSource::for_test(consumer, 4);
        let mut out = vec![0.0f32; 12]; // drains the 6 frames, then underruns
        src.read(&mut out, 2);
        assert_eq!(src.rebuffer_count(), 1, "draining mid-track arms one rebuffer");
    }

    /// googlevideo paces a plain GET to ~1× realtime; the same request carrying
    /// a Range serves the same body ~190× faster. The gapless path learned this
    /// (see `stream_queue.rs`); this is the progressive path's copy of the same
    /// lesson — the FIRST open must ask as a range too, because that first open
    /// is the one the listener is waiting on.
    #[test]
    fn the_first_open_asks_for_the_body_as_a_range() {
        use std::io::{BufRead, BufWriter, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));

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
            let body = b"bytes";
            let mut w = BufWriter::new(stream);
            let _ = write!(
                w,
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-4/5\r\n\
                 Content-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = w.write_all(body);
            let _ = w.flush();
        });

        let client = reqwest::blocking::Client::new();
        let r = open(
            &client,
            &format!("http://{addr}/track"),
            &[("User-Agent".into(), "hm-test".into())],
            0,
        );
        server.join().expect("server thread");
        assert!(r.is_some(), "a 206 at byte 0 must be accepted");

        let lines = seen.lock().unwrap().clone();
        let range = lines
            .iter()
            .find(|l| l.to_ascii_lowercase().starts_with("range:"))
            .unwrap_or_else(|| panic!("no Range header was sent; got {lines:#?}"));
        assert!(
            range.eq_ignore_ascii_case("range: bytes=0-"),
            "asked for the wrong range: {range}"
        );
    }

    /// Some radio/Icecast servers ignore Range and answer 200 — at byte 0
    /// that is exactly the body we asked for, so it must keep working.
    #[test]
    fn a_200_at_byte_zero_is_still_accepted() {
        use std::io::{BufRead, BufWriter, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
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
            }
            let body = b"bytes";
            let mut w = BufWriter::new(stream);
            let _ = write!(
                w,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = w.write_all(body);
            let _ = w.flush();
        });

        let client = reqwest::blocking::Client::new();
        let r = open(&client, &format!("http://{addr}/live"), &[], 0);
        server.join().expect("server thread");
        assert!(r.is_some(), "a Range-ignoring server must not break byte-0 opens");
    }

    /// At an offset a 200 means the server would replay from byte 0 —
    /// audible duplication. That rejection must survive this change.
    #[test]
    fn a_200_at_an_offset_is_still_rejected() {
        use std::io::{BufRead, BufWriter, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
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
            }
            let body = b"bytes";
            let mut w = BufWriter::new(stream);
            let _ = write!(
                w,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = w.write_all(body);
            let _ = w.flush();
        });

        let client = reqwest::blocking::Client::new();
        let r = open(&client, &format!("http://{addr}/track"), &[], 4096);
        server.join().expect("server thread");
        assert!(r.is_none(), "a replay-from-zero response must be treated as a failed open");
    }
}
