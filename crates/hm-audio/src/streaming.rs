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

/// An HTTP audio stream rendered as an [`AudioSource`].
pub struct RadioStreamSource {
    consumer: rtrb::Consumer<f32>,
    shared: Arc<StreamShared>,
    device_rate: u32,
    _thread: JoinHandle<()>,
}

impl RadioStreamSource {
    /// Start streaming `url`, producing stereo at `device_rate`.
    pub fn new(url: String, device_rate: u32) -> Self {
        Self::with_headers(url, Vec::new(), device_rate, None, None)
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
    ) -> Self {
        // ~8 seconds of stereo headroom.
        let capacity = (device_rate.max(8_000) as usize) * 2 * 8;
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
            _thread: thread,
        }
    }
}

impl Drop for RadioStreamSource {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::Relaxed);
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

    loop {
        if !shared.running.load(Ordering::Relaxed) {
            return;
        }

        let Some(response) = open(&client, url, headers, start_byte) else {
            // Couldn't (re)open. If we had already started, fall back to the
            // beginning once; otherwise give up.
            if start_byte > 0 {
                start_byte = 0;
                continue;
            }
            return;
        };
        record_content_length(&shared, &response, start_byte);

        let sink = if meta_published {
            None
        } else {
            meta_sink.clone()
        };
        let stop = decode_connection(
            response,
            device_rate,
            &mut producer,
            &shared,
            sink,
            duration_hint,
            start_byte,
            ext.as_deref(),
            &mut meta_published,
        );

        match stop {
            Stop::Cancelled => return,
            Stop::Seek(target) => {
                start_byte = byte_offset(&shared, device_rate, target);
                shared.finished.store(false, Ordering::Relaxed);
            }
            Stop::Eof => {
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
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        }
    }
}

/// Bytes to fetch when reading only a remote file's metadata. ID3v2 / FLAC /
/// front-loaded MP4 headers (incl. embedded cover art) live in the leading
/// bytes, so this bounds the download — pre-play metadata never pulls the whole
/// track.
const META_PREFIX_BYTES: u64 = 2 * 1024 * 1024;

/// Read title/artist/album + embedded cover from a remote audio file by fetching
/// only its leading [`META_PREFIX_BYTES`], without streaming the whole file — the
/// desktop counterpart to the mobile app's `MediaMetadataRetriever`-over-URL
/// cloud metadata. `ext` hints the container (from the file name). Returns
/// `None` if the file can't be fetched or probed.
pub fn fetch_stream_metadata(
    url: &str,
    headers: &[(String, String)],
    ext: Option<&str>,
) -> Option<hm_core::TrackMeta> {
    use std::io::Read;

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(6))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .ok()?;
    let mut req = client
        .get(url)
        .header("Range", format!("bytes=0-{}", META_PREFIX_BYTES - 1));
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = match req.send() {
        Ok(r) if r.status().is_success() => r,
        _ => return None,
    };

    // Cap the read in case the server ignored the Range request.
    let capped = resp.take(META_PREFIX_BYTES);
    let mss = MediaSourceStream::new(Box::new(ReadOnlySource::new(capped)), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .ok()?;
    Some(crate::meta::extract_metadata(&mut *format))
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
    device_rate: u32,
    producer: &mut Producer<f32>,
    shared: &StreamShared,
    meta_sink: Option<crate::engine::MetaSink>,
    duration_hint: Option<f64>,
    start_byte: u64,
    ext: Option<&str>,
    meta_published: &mut bool,
) -> Stop {
    let mss = MediaSourceStream::new(Box::new(ReadOnlySource::new(response)), Default::default());
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
mod tests {
    use super::*;

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
}
