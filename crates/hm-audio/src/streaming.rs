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

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
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
        if !finished && self.consumer.slots() < 2 {
            self.buffering = true; // ring drained mid-track → rebuffer next block
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
}

/// Why a single connection's decode loop stopped.
enum Stop {
    /// Streaming was cancelled (source dropped / stopped).
    Cancelled,
    /// The stream was fully consumed.
    Eof,
    /// A seek to `device-rate frame` was requested.
    Seek(u64),
}

/// How many no-progress reconnects in a row we tolerate before giving up on a
/// track (so a server that keeps closing, or a container we can't re-probe
/// mid-file, ends the track instead of hot-looping).
const MAX_STALLS: u32 = 3;

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
    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(12))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    // Extension hint helps symphonia pick a demuxer when content sniffing alone
    // is ambiguous (e.g. raw AAC/MP3).
    let ext = url
        .split(['?', '#'])
        .next()
        .and_then(|p| p.rsplit('.').next())
        .map(str::to_ascii_lowercase)
        .filter(|e| matches!(e.as_str(), "mp3" | "aac" | "ogg" | "flac" | "m4a" | "mp4" | "wav"));

    let mut start_byte = 0u64;
    let mut meta_published = false;
    let mut stalls = 0u32;
    let conn_bytes = Arc::new(AtomicU64::new(0));
    let mut connect_fails = 0u32;

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
                std::thread::sleep(Duration::from_millis(400 * connect_fails as u64));
                continue;
            }
            if start_byte > 0 {
                start_byte = 0;
                connect_fails = 0;
                continue;
            }
            return;
        };
        connect_fails = 0;
        record_content_length(&shared, &response, start_byte);

        let sink = if meta_published { None } else { meta_sink.clone() };
        let stop = decode_connection(
            response,
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

        let progressed = conn_bytes.load(Ordering::Relaxed) > 0;
        let consumed = start_byte + conn_bytes.load(Ordering::Relaxed);

        match stop {
            Stop::Cancelled => return,
            Stop::Seek(target) => {
                start_byte = byte_offset(&shared, device_rate, target);
                shared.finished.store(false, Ordering::Relaxed);
                stalls = 0;
            }
            Stop::Eof => {
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
    let resp = meta_request(client, url, headers, max_bytes)?;
    let bytes = resp.bytes().ok()?.to_vec();
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

/// Issue the GET, optionally from `start_byte` via a `Range` header.
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
    if start_byte > 0 {
        req = req.header("Range", format!("bytes={start_byte}-"));
    }
    match req.send() {
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

/// Probe + decode a single HTTP response, pushing resampled stereo into the
/// ring. Returns why it stopped. On the initial open (`start_byte == 0`) it also
/// learns the duration and publishes track metadata.
#[allow(clippy::too_many_arguments)]
fn decode_connection(
    response: reqwest::blocking::Response,
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
    let counted = CountingReader { inner: response, count: conn_bytes };
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
        // Mid-stream probe can fail for container formats; give up this attempt.
        return Stop::Eof;
    };

    if let Some(sink) = &meta_sink {
        sink.set(crate::meta::extract_metadata(&mut *format));
        *meta_published = true;
    }

    let Some(track) = format.default_track(TrackType::Audio) else {
        return Stop::Eof;
    };
    let track_id = track.id;
    let Some(params) = track.codec_params.as_ref().and_then(|c| c.audio()).cloned() else {
        return Stop::Eof;
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
        return Stop::Eof;
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
            _ => return Stop::Eof,
        };
        if packet.track_id != track_id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(a) => a,
            Err(SymError::DecodeError(_)) => continue,
            Err(_) => return Stop::Eof,
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

    #[test]
    fn counting_reader_counts_bytes_read() {
        use std::io::Read;
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
}
