//! Streamed gapless / crossfade queue (cloud & phone).
//!
//! Mirrors [`QueuePlaybackSource`](crate::queue::QueuePlaybackSource) but decodes
//! each track by **streaming it over the network** rather than reading a local
//! file, and resolves each track's URL **lazily** — just before it's needed —
//! via a caller-supplied [`StreamResolver`]. That matters because some providers
//! (e.g. Dropbox) cost an API call per URL and hand out short-lived links, so
//! resolving the whole queue up front would be slow and the links could go stale.
//!
//! Only the current track and a one-track lookahead are held decoded in memory;
//! tracks already played are freed. So a long cloud/phone queue never downloads
//! or decodes everything at once. The crossfade ramp itself matches the local
//! queue exactly.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use hm_core::TrackMeta;

use crate::decode::{decode_bytes, resample_stereo};
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
pub type StreamResolver = Arc<dyn Fn(usize) -> Result<StreamTarget, String> + Send + Sync>;

/// `(absolute index, decoded interleaved stereo, metadata)` from the worker.
type DecodedTrack = (usize, Vec<f32>, TrackMeta);

/// Fetch + decode one streamed track fully, resampled to `device_rate`.
fn decode_stream(target: &StreamTarget, device_rate: u32) -> Result<(Vec<f32>, TrackMeta), AudioError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| AudioError::Stream(e.to_string()))?;
    let mut req = client.get(&target.url);
    for (k, v) in &target.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().map_err(|e| AudioError::Stream(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(AudioError::Stream(format!(
            "stream server returned HTTP {}",
            resp.status().as_u16()
        )));
    }
    let bytes = resp
        .bytes()
        .map_err(|e| AudioError::Stream(e.to_string()))?
        .to_vec();
    let decoded = decode_bytes(bytes, target.ext.as_deref())?;
    Ok((
        resample_stereo(&decoded.samples, decoded.sample_rate, device_rate),
        decoded.meta,
    ))
}

/// A queue of streamed tracks played gaplessly, with optional crossfade.
pub struct StreamQueueSource {
    /// Decoded tracks by absolute index. `None` = not decoded yet (buffer
    /// silence), `Some(empty)` = decoded-but-failed (skip), `Some(samples)` =
    /// ready. Tracks below the play head are freed back to `None`.
    tracks: Vec<Option<Vec<f32>>>,
    metas: Vec<TrackMeta>,
    count: usize,
    device_rate: u32,
    crossfade: Arc<AtomicU32>,
    index: usize,
    cursor: usize,
    xf_cursor: usize,
    rx: Option<Receiver<DecodedTrack>>,
    /// Asks the worker to ensure tracks are decoded up to (and incl.) this index.
    want_tx: Option<Sender<usize>>,
    running: Arc<AtomicBool>,
    meta_sink: Option<MetaSink>,
    current_index: Option<Arc<AtomicUsize>>,
    _worker: Option<JoinHandle<()>>,
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
        let (tx, rx) = mpsc::channel();
        let (want_tx, want_rx) = mpsc::channel::<usize>();
        let running = Arc::new(AtomicBool::new(true));

        let worker = {
            let running = running.clone();
            std::thread::Builder::new()
                .name("hm-stream-queue".into())
                .spawn(move || {
                    let mut next = start;
                    while let Ok(want) = want_rx.recv() {
                        while next <= want && next < count {
                            if !running.load(Ordering::Relaxed) {
                                return;
                            }
                            let decoded =
                                resolver(next).and_then(|t| {
                                    decode_stream(&t, device_rate).map_err(|e| e.to_string())
                                });
                            // Failed resolve/decode → empty entry the source skips.
                            let track = match decoded {
                                Ok((samples, meta)) => (next, samples, meta),
                                Err(_) => (next, Vec::new(), TrackMeta::default()),
                            };
                            if tx.send(track).is_err() {
                                return;
                            }
                            next += 1;
                        }
                    }
                })
                .ok()
        };

        let mut tracks = Vec::with_capacity(count);
        tracks.resize_with(count, || None);
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
            rx: Some(rx),
            want_tx: Some(want_tx),
            running,
            meta_sink,
            current_index,
            _worker: worker,
        }
    }

    fn ready(&self, i: usize) -> bool {
        self.tracks.get(i).is_some_and(|t| t.is_some())
    }

    fn track_len(&self, i: usize) -> usize {
        self.tracks
            .get(i)
            .and_then(|t| t.as_ref())
            .map_or(0, |t| t.len() / 2)
    }

    fn frame(&self, i: usize, f: usize) -> (f32, f32) {
        match self.tracks.get(i).and_then(|t| t.as_ref()) {
            Some(t) if f * 2 + 1 < t.len() => (t[f * 2], t[f * 2 + 1]),
            _ => (0.0, 0.0),
        }
    }

    fn crossfade_frames(&self) -> usize {
        let secs = f32::from_bits(self.crossfade.load(Ordering::Relaxed)).max(0.0);
        (secs * self.device_rate as f32).round() as usize
    }

    /// Pull any newly-decoded tracks from the worker into the window.
    fn drain(&mut self) {
        if let Some(rx) = &self.rx {
            while let Ok((idx, samples, meta)) = rx.try_recv() {
                if idx < self.count {
                    self.tracks[idx] = Some(samples);
                    self.metas[idx] = meta;
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
                *slot = None;
            }
        }
    }

    fn signal_track(&self) {
        if let Some(idx) = &self.current_index {
            idx.store(self.index, Ordering::Release);
        }
        if let Some(sink) = &self.meta_sink {
            if let Some(meta) = self.metas.get(self.index) {
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
            let next_ready = self.index + 1 < self.count && self.ready(self.index + 1);
            let xf = self.crossfade_frames();
            let crossfading = xf > 0 && next_ready && self.cursor + xf >= cur_len;

            let (l, r) = if crossfading {
                let t = (self.xf_cursor as f32 / xf as f32).min(1.0);
                let (lc, rc) = self.frame(self.index, self.cursor);
                let (ln, rn) = self.frame(self.index + 1, self.xf_cursor);
                self.cursor += 1;
                self.xf_cursor += 1;
                if self.cursor >= cur_len {
                    self.index += 1;
                    self.cursor = self.xf_cursor;
                    self.xf_cursor = 0;
                    self.signal_track();
                    self.advance_window();
                }
                (lc * (1.0 - t) + ln * t, rc * (1.0 - t) + rn * t)
            } else if self.cursor < cur_len {
                let frame = self.frame(self.index, self.cursor);
                self.cursor += 1;
                frame
            } else {
                // Gapless boundary: advance to the next track.
                self.index += 1;
                self.cursor = 0;
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
    }

    fn position(&self) -> usize {
        self.cursor.min(self.track_len(self.index))
    }

    fn total_frames(&self) -> usize {
        self.track_len(self.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo(value: f32, frames: usize) -> Vec<f32> {
        vec![value; frames * 2]
    }

    /// An eager source over pre-decoded tracks (no worker), for testing the
    /// gapless/crossfade read logic. `crossfade_frames` maps directly to frames
    /// here (device_rate = 1).
    fn eager(tracks: Vec<Vec<f32>>, crossfade_frames: usize) -> StreamQueueSource {
        let count = tracks.len();
        StreamQueueSource {
            tracks: tracks.into_iter().map(Some).collect(),
            metas: vec![TrackMeta::default(); count],
            count,
            device_rate: 1,
            crossfade: Arc::new(AtomicU32::new((crossfade_frames as f32).to_bits())),
            index: 0,
            cursor: 0,
            xf_cursor: 0,
            rx: None,
            want_tx: None,
            running: Arc::new(AtomicBool::new(true)),
            meta_sink: None,
            current_index: None,
            _worker: None,
        }
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

    #[test]
    fn crossfade_ramps_from_one_track_to_the_next() {
        let mut src = eager(vec![stereo(1.0, 4), stereo(0.0, 8)], 4);
        let mut out = vec![0.0f32; 8]; // the 4 crossfade frames
        src.read(&mut out, 2);
        let left = [out[0], out[2], out[4], out[6]];
        for (got, want) in left.iter().zip([1.0, 0.75, 0.5, 0.25]) {
            assert!((got - want).abs() < 1e-6, "crossfade ramp: {got} vs {want}");
        }
    }

    #[test]
    fn buffers_silence_while_a_track_is_undecoded() {
        // Track 0 not yet decoded (None): produce silence but NOT end-of-stream.
        let mut src = eager(vec![stereo(0.5, 4)], 0);
        src.tracks[0] = None;
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
}
