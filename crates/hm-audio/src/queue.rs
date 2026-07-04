//! Gapless / crossfade queue playback.
//!
//! Plays a list of local stereo tracks (at the device rate) back-to-back with
//! **no gap**, optionally **crossfading** between them. Tracks are decoded on a
//! background worker that is **demand-gated**: it only keeps the current track
//! plus a one-track lookahead decoded, and tracks already played are freed, so
//! playing a long queue never holds the whole library's decoded PCM in RAM
//! (which at ~92 MB per 4-min track would otherwise reach many GB). It reports
//! the current track's metadata (via a [`MetaSink`]) and absolute queue index
//! (via an atomic) on each transition; `position`/`total_frames` report the
//! *current* track so the seek bar stays per-track.
//!
//! This is the local-file sibling of [`StreamQueueSource`](crate::stream_queue::StreamQueueSource)
//! (cloud/phone) and shares the same bounded-window read/crossfade logic; the
//! only difference is decoding a local file instead of streaming over the network.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use hm_core::TrackMeta;

use crate::decode::{decode_file, resample_stereo};
use crate::engine::MetaSink;
use crate::error::AudioError;
use crate::{AudioSource, StreamFormat};

/// `(index, decoded interleaved stereo, metadata)` handed from the decode worker
/// to the source. The index lets the source place a track in its fixed-size
/// window even though tracks decode in order.
type DecodedTrack = (usize, Vec<f32>, TrackMeta);

/// A queue of tracks played gaplessly, with optional crossfade.
pub struct QueuePlaybackSource {
    /// Decoded tracks by index. `None` = not decoded yet (buffer silence) or
    /// freed after playing, `Some(empty)` = decoded-but-failed (skip),
    /// `Some(samples)` = ready. Tracks below the play head are freed back to
    /// `None`, so memory stays bounded to ~2 tracks.
    tracks: Vec<Option<Vec<f32>>>,
    metas: Vec<TrackMeta>,
    /// Total number of tracks in the queue (the window is this long, mostly `None`).
    expected: usize,
    /// Decoded tracks arriving from the background worker (`None` when eager).
    rx: Option<Receiver<DecodedTrack>>,
    /// Asks the worker to ensure tracks are decoded up to (and incl.) this index.
    want_tx: Option<Sender<usize>>,
    /// Live crossfade duration in seconds (f32 bits), shared with the engine so
    /// slider changes apply to this queue's upcoming transitions.
    crossfade: Arc<AtomicU32>,
    /// Output sample rate, to convert the crossfade seconds to frames.
    device_rate: u32,
    /// Absolute queue index of local track 0 (so reporting maps back).
    index_offset: usize,
    index: usize,
    cursor: usize,
    /// Frame offset into the *incoming* track while a crossfade is in progress.
    xf_cursor: usize,
    running: Arc<AtomicBool>,
    meta_sink: Option<MetaSink>,
    current_index: Option<Arc<AtomicUsize>>,
    _worker: Option<JoinHandle<()>>,
}

impl QueuePlaybackSource {
    /// Spawn a background decoder for `paths` and play them as a gapless queue.
    /// `start` is the index within `paths` to begin at; `crossfade_secs > 0`
    /// crossfades between tracks. Reports progress via `meta_sink`/`current_index`.
    /// Only the current track plus a one-track lookahead are ever held decoded.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        paths: Vec<String>,
        start: usize,
        device_rate: u32,
        crossfade: Arc<AtomicU32>,
        meta_sink: Option<MetaSink>,
        current_index: Option<Arc<AtomicUsize>>,
    ) -> Self {
        let start = start.min(paths.len().saturating_sub(1));
        // Local track 0 == `paths[start]`; everything below is 0-based from here,
        // with `index_offset` mapping back to the absolute queue index.
        let queue: Vec<String> = paths.into_iter().skip(start).collect();
        let expected = queue.len();
        let (tx, rx) = mpsc::channel();
        let (want_tx, want_rx) = mpsc::channel::<usize>();
        let running = Arc::new(AtomicBool::new(true));

        let worker = {
            let running = running.clone();
            std::thread::Builder::new()
                .name("hm-queue-decode".into())
                .spawn(move || {
                    // Whole-track decode+resample runs in bursts; keep it from
                    // competing with the audio callback (which is what glitches
                    // on 2-core machines exactly at track transitions).
                    crate::thread_util::lower_current_thread_priority();
                    // Demand-gated: only decode up to the index the source has
                    // asked for (current + one lookahead). This is what bounds
                    // memory — without it the worker would race ahead and decode
                    // the whole queue into RAM at once.
                    let mut next = 0;
                    while let Ok(want) = want_rx.recv() {
                        while next <= want && next < expected {
                            if !running.load(Ordering::Relaxed) {
                                return;
                            }
                            let idx = next;
                            let decoded = decode_file(Path::new(&queue[idx])).ok();
                            let track = match decoded {
                                Some(d) => (
                                    idx,
                                    resample_stereo(&d.samples, d.sample_rate, device_rate),
                                    d.meta,
                                ),
                                // A track that fails to decode becomes an empty
                                // (zero-length) entry, which the source skips instantly.
                                None => (idx, Vec::new(), TrackMeta::default()),
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

        let mut tracks = Vec::with_capacity(expected);
        tracks.resize_with(expected, || None);
        let metas = vec![TrackMeta::default(); expected];

        // Prime the current track + one lookahead.
        let _ = want_tx.send(1);

        Self {
            tracks,
            metas,
            expected,
            rx: Some(rx),
            want_tx: Some(want_tx),
            crossfade,
            device_rate,
            index_offset: start,
            index: 0,
            cursor: 0,
            xf_cursor: 0,
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

    /// Current crossfade length in frames, read live from the shared value.
    fn crossfade_frames(&self) -> usize {
        let secs = f32::from_bits(self.crossfade.load(Ordering::Relaxed)).max(0.0);
        (secs * self.device_rate as f32).round() as usize
    }

    /// One stereo frame from track `i`, or silence past its end / if not decoded.
    fn frame(&self, i: usize, f: usize) -> (f32, f32) {
        match self.tracks.get(i).and_then(|t| t.as_ref()) {
            Some(t) if f * 2 + 1 < t.len() => (t[f * 2], t[f * 2 + 1]),
            _ => (0.0, 0.0),
        }
    }

    /// Pull any newly-decoded tracks from the worker into the window.
    fn drain(&mut self) {
        if let Some(rx) = &self.rx {
            while let Ok((idx, samples, meta)) = rx.try_recv() {
                if idx < self.expected {
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
            let _ = tx.send((self.index + 1).min(self.expected.saturating_sub(1)));
        }
        for i in 0..self.index {
            if let Some(slot) = self.tracks.get_mut(i) {
                *slot = None;
            }
        }
    }

    /// Report the current track to the UI (absolute index + its metadata).
    fn signal_track(&self) {
        if let Some(idx) = &self.current_index {
            idx.store(self.index_offset + self.index, Ordering::Release);
        }
        if let Some(sink) = &self.meta_sink {
            if let Some(meta) = self.metas.get(self.index) {
                sink.set(meta.clone());
            }
        }
    }
}

impl Drop for QueuePlaybackSource {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

pub(crate) fn write_frame(out: &mut [f32], base: usize, channels: usize, l: f32, r: f32) {
    if channels == 1 {
        out[base] = 0.5 * (l + r);
    } else {
        out[base] = l;
        out[base + 1] = r;
        for ch in out.iter_mut().take(base + channels).skip(base + 2) {
            *ch = 0.0;
        }
    }
}

impl AudioSource for QueuePlaybackSource {
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
            if self.index >= self.expected {
                write_frame(out, base, channels, 0.0, 0.0);
                continue;
            }
            // Current track still decoding: buffer (silence, not EOF).
            if !self.ready(self.index) {
                write_frame(out, base, channels, 0.0, 0.0);
                produced += 1;
                continue;
            }

            let cur_len = self.track_len(self.index);
            let next_ready = self.index + 1 < self.expected && self.ready(self.index + 1);
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
                } else if self.index < self.expected {
                    self.signal_track(); // index it, even if still buffering
                    (0.0, 0.0)
                } else {
                    (0.0, 0.0)
                }
            };

            write_frame(out, base, channels, l, r);
            if self.index < self.expected {
                produced += 1;
            }
        }
        produced
    }

    fn stop(&mut self) {
        self.index = self.expected;
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

    /// Build an eager (no worker) source over pre-decoded tracks, for testing the
    /// gapless/crossfade read logic. `crossfade_frames` maps directly to frames
    /// here (device_rate = 1).
    fn eager(tracks: Vec<Vec<f32>>, crossfade_frames: usize) -> QueuePlaybackSource {
        let expected = tracks.len();
        QueuePlaybackSource {
            tracks: tracks.into_iter().map(Some).collect(),
            metas: vec![TrackMeta::default(); expected],
            expected,
            rx: None,
            want_tx: None,
            crossfade: Arc::new(AtomicU32::new((crossfade_frames as f32).to_bits())),
            device_rate: 1,
            index_offset: 0,
            index: 0,
            cursor: 0,
            xf_cursor: 0,
            running: Arc::new(AtomicBool::new(true)),
            meta_sink: None,
            current_index: None,
            _worker: None,
        }
    }

    #[test]
    fn plays_tracks_gaplessly_then_ends() {
        let tracks = vec![vec![0.1, 0.1, 0.2, 0.2], vec![0.3, 0.3, 0.4, 0.4]];
        let mut src = eager(tracks, 0);

        let mut out = vec![0.0f32; 10]; // 5 stereo frames
        let produced = src.read(&mut out, 2);
        assert_eq!(produced, 4, "4 real frames across both tracks");
        assert_eq!(&out[0..8], &[0.1, 0.1, 0.2, 0.2, 0.3, 0.3, 0.4, 0.4]);
        assert_eq!(&out[8..10], &[0.0, 0.0], "silence past the end");

        let mut tail = vec![0.0f32; 4];
        assert_eq!(src.read(&mut tail, 2), 0, "EOF after the queue is exhausted");
    }

    #[test]
    fn crossfade_ramps_from_one_track_to_the_next() {
        let tracks = vec![stereo(1.0, 4), stereo(0.0, 8)];
        let mut src = eager(tracks, 4);

        let mut out = vec![0.0f32; 8]; // the 4 crossfade frames
        src.read(&mut out, 2);
        let left = [out[0], out[2], out[4], out[6]];
        for (got, want) in left.iter().zip([1.0, 0.75, 0.5, 0.25]) {
            assert!((got - want).abs() < 1e-6, "crossfade ramp: {got} vs {want}");
        }
    }

    #[test]
    fn reports_current_track_position_and_total() {
        let mut src = eager(vec![stereo(0.5, 3), stereo(0.5, 5)], 0);
        assert_eq!(src.total_frames(), 3);
        let mut out = vec![0.0f32; 8]; // 4 frames: exhausts track 0, enters track 1
        src.read(&mut out, 2);
        assert_eq!(src.index, 1);
        assert_eq!(src.total_frames(), 5, "now reports track 1's length");
    }

    #[test]
    fn buffers_silence_while_a_track_is_undecoded() {
        // Current track not yet decoded (None): produce silence but NOT EOF.
        let mut src = eager(vec![stereo(0.5, 4)], 0);
        src.tracks[0] = None;
        let mut out = vec![0.0f32; 6];
        let produced = src.read(&mut out, 2);
        assert_eq!(produced, 3, "still 'producing' (buffering), not EOF");
        assert!(out.iter().all(|&s| s == 0.0), "all silence while buffering");
    }

    #[test]
    fn frees_played_tracks_to_bound_memory() {
        // After advancing off track 0, its decoded PCM is freed; the current
        // track (1) and lookahead stay resident — this is the memory bound.
        let mut src = eager(vec![vec![0.1, 0.1], vec![0.2, 0.2], vec![0.3, 0.3]], 0);
        let mut out = vec![0.0f32; 4]; // 2 frames: play track 0, cross into track 1
        src.read(&mut out, 2);
        assert_eq!(src.index, 1, "now on track 1");
        assert!(src.tracks[0].is_none(), "played track 0 freed");
        assert!(src.tracks[1].is_some(), "current track 1 still resident");
    }
}
