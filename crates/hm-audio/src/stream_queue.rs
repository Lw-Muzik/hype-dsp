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

use crate::decode::{decode_file, resample_stereo};
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

/// `(absolute index, decoded interleaved stereo, metadata)` from the worker.
type DecodedTrack = (usize, Vec<f32>, TrackMeta);

/// How many times the worker tries to fetch + decode one track before giving up
/// and skipping it. Transient failures — a connection dropped while the stream
/// sat paused, a flaky cloud link, a phone that closed its keep-alive, a 5xx —
/// are retried on a fresh connection; a permanent failure (404/403, an
/// undecodable body) is skipped at once without burning the retry budget. This
/// is what stops one stale connection from silently nuking a good track (which
/// looked like the queue "jumping" or "stopping" on its own).
const MAX_ATTEMPTS: u32 = 4;

/// Build a fresh blocking HTTP client. The short `connect_timeout` matters: a
/// dead pooled connection (common after the stream has been paused a while)
/// then fails fast and is retried, rather than stalling playback on a long hang.
fn new_client() -> reqwest::Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(90))
        .build()
}

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

/// The outcome of a single fetch attempt, classified so the retry loop knows
/// whether trying again could possibly help.
enum FetchOutcome {
    /// Got the full body, spooled to a temp file (deleted when the guard drops).
    Body(SpooledBody),
    /// Permanent failure (4xx) — retrying the same URL won't help; skip it.
    Skip,
    /// Transient failure (connect/read error, timeout, 408/429/5xx, a truncated
    /// body) — worth retrying on a fresh connection.
    Retry,
}

/// Issue one GET for `target`, stream the body to a temp file, and classify the
/// outcome. Streaming (rather than `resp.bytes()`) keeps the compressed file
/// out of RAM; a spool I/O error is treated exactly like a failed body read —
/// a transient failure worth retrying.
fn fetch_once(client: &reqwest::blocking::Client, target: &StreamTarget) -> FetchOutcome {
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
    match req.send() {
        Ok(mut resp) => {
            let status = resp.status();
            if status.is_success() {
                // A reset/truncated body surfaces here as an error — retry it,
                // because a partial download would otherwise decode to a short
                // track that ends early (and "jumps" to the next one).
                let expected = resp.content_length();
                let (spool, mut file) = match SpooledBody::create(target.ext.as_deref()) {
                    Ok(v) => v,
                    Err(_) => return FetchOutcome::Retry,
                };
                match std::io::copy(&mut resp, &mut file) {
                    // Belt-and-suspenders: if the server declared a length and we
                    // got fewer bytes, treat it as truncated and retry rather than
                    // decode a clipped track.
                    Ok(written) if expected.is_none_or(|n| written >= n) => {
                        FetchOutcome::Body(spool)
                    }
                    Ok(_) | Err(_) => FetchOutcome::Retry, // spool guard cleans up
                }
            } else if status.is_server_error()
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::REQUEST_TIMEOUT
            {
                FetchOutcome::Retry
            } else {
                FetchOutcome::Skip
            }
        }
        // Connect/timeout/transport error — the pooled connection may be stale.
        Err(_) => FetchOutcome::Retry,
    }
}

/// One attempt's classified result, decode included.
enum LoadAttempt {
    /// Decoded + resampled interleaved stereo, plus its metadata.
    Ready(Vec<f32>, TrackMeta),
    /// Give up on this track (permanent failure) — it becomes a silent skip.
    Skip,
    /// Transient failure — try again.
    Retry,
}

/// Run `attempt` up to [`MAX_ATTEMPTS`] times, sleeping `backoff` (doubling, to a
/// 2 s cap) between transient retries, and return the decoded track. A permanent
/// `Skip`, an exhausted retry budget, or a stop request yields an empty track
/// (which the source skips). `backoff` is a parameter so tests can pass zero.
fn load_with_retry(
    index: usize,
    running: &AtomicBool,
    backoff: Duration,
    mut attempt: impl FnMut(u32) -> LoadAttempt,
) -> DecodedTrack {
    let mut wait = backoff;
    for n in 1..=MAX_ATTEMPTS {
        if !running.load(Ordering::Relaxed) {
            break;
        }
        match attempt(n) {
            LoadAttempt::Ready(samples, meta) => return (index, samples, meta),
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
    (index, Vec::new(), TrackMeta::default())
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
    /// Frames the in-progress crossfade ramps over, latched when it starts;
    /// 0 when not crossfading.
    ///
    /// Not simply `crossfade_frames()`: a lookahead that finishes decoding after
    /// the window has already opened leaves fewer frames than that, and ramping
    /// against the full width would leave the outgoing track at audible gain
    /// when it's cut. Which is to say the click would land exactly when the
    /// network is slowest — the case crossfade most needs to survive.
    xf_len: usize,
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
                    // One HTTP client reused across tracks (each blocking client
                    // spins up its own runtime, so we avoid rebuilding per track),
                    // but replaced after a transient failure to drop a stale pool.
                    let mut client = match new_client() {
                        Ok(c) => c,
                        Err(_) => return,
                    };
                    let mut next = start;
                    while let Ok(want) = want_rx.recv() {
                        while next <= want && next < count {
                            if !running.load(Ordering::Relaxed) {
                                return;
                            }
                            let idx = next;
                            // Retry transient failures on a new connection, so a
                            // dropped link — e.g. the first fetch after the stream
                            // sat paused, or a phone that closed its keep-alive —
                            // doesn't turn a good track into a permanent silent
                            // skip. Only a retry demands a fresh link: asking for
                            // one on the first attempt would make every provider
                            // that can cache pay to resolve anyway, and for YT
                            // Music resolving is a yt-dlp process start measured
                            // in seconds — the gap between two tracks.
                            let track = load_with_retry(
                                idx,
                                &running,
                                Duration::from_millis(120),
                                |n| match resolver(idx, n > 1) {
                                    Err(_) => {
                                        if let Ok(c) = new_client() {
                                            client = c;
                                        }
                                        LoadAttempt::Retry
                                    }
                                    Ok(target) => match fetch_once(&client, &target) {
                                        // Decode straight from the spool file
                                        // (deleted when `spool` drops, on every
                                        // path) so only the decoded PCM — never
                                        // the compressed body — lives in RAM.
                                        FetchOutcome::Body(spool) => {
                                            match decode_file(&spool.path) {
                                                Ok(d) => {
                                                    let samples = resample_stereo(
                                                        &d.samples,
                                                        d.sample_rate,
                                                        device_rate,
                                                    );
                                                    if samples.is_empty() {
                                                        LoadAttempt::Skip
                                                    } else {
                                                        LoadAttempt::Ready(samples, d.meta)
                                                    }
                                                }
                                                Err(_) => LoadAttempt::Skip,
                                            }
                                        }
                                        FetchOutcome::Skip => LoadAttempt::Skip,
                                        FetchOutcome::Retry => {
                                            if let Ok(c) = new_client() {
                                                client = c;
                                            }
                                            LoadAttempt::Retry
                                        }
                                    },
                                },
                            );
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
            xf_len: 0,
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
            let next_ready = self.index + 1 < self.count && self.ready(self.index + 1);
            let xf = self.crossfade_frames();
            let crossfading = xf > 0 && next_ready && self.cursor + xf >= cur_len;

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
            } else {
                // Gapless boundary: advance to the next track.
                self.index += 1;
                self.cursor = 0;
                self.xf_len = 0;
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
        // The body is nonsense, so this can only get as far as spooling it —
        // which is all this test is about. Decoding is someone else's test.
        let _ = fetch_once(&client, &target);
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
        src.tracks[1] = None; // still decoding

        // Play to frame 80 — 20 frames INTO the window with nothing to fade to.
        let mut out = vec![0.0f32; 160];
        src.read(&mut out, 2);
        assert!(
            out.iter().all(|s| (*s - 1.0).abs() < 1e-6),
            "current track should play untouched while there's nothing to fade to"
        );

        // It lands now, with 20 of the current track's frames left.
        src.tracks[1] = Some(stereo(-1.0, 100));

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

    #[test]
    fn retry_recovers_a_transient_failure() {
        // A flaky fetch (e.g. a stale connection after a pause) fails twice then
        // succeeds — the track must still load, not become a silent skip.
        let running = AtomicBool::new(true);
        let mut calls = 0u32;
        let track = load_with_retry(7, &running, Duration::ZERO, |_| {
            calls += 1;
            if calls < 3 {
                LoadAttempt::Retry
            } else {
                LoadAttempt::Ready(vec![0.2, 0.2], TrackMeta::default())
            }
        });
        assert_eq!(track.0, 7);
        assert_eq!(track.1, vec![0.2, 0.2], "track recovered after retries");
        assert_eq!(calls, 3, "kept trying until it succeeded");
    }

    #[test]
    fn permanent_failure_skips_without_retrying() {
        // A 404-class failure shouldn't burn the retry budget — skip immediately.
        let running = AtomicBool::new(true);
        let mut calls = 0u32;
        let track = load_with_retry(2, &running, Duration::ZERO, |_| {
            calls += 1;
            LoadAttempt::Skip
        });
        assert!(track.1.is_empty(), "permanent failure → empty (skipped) track");
        assert_eq!(calls, 1, "no retries for a permanent failure");
    }

    #[test]
    fn gives_up_after_max_attempts() {
        // Endlessly transient → bounded retries, then a silent skip (so the queue
        // moves on instead of buffering forever).
        let running = AtomicBool::new(true);
        let mut calls = 0u32;
        let track = load_with_retry(0, &running, Duration::ZERO, |_| {
            calls += 1;
            LoadAttempt::Retry
        });
        assert!(track.1.is_empty(), "exhausted retries → skip");
        assert_eq!(calls, MAX_ATTEMPTS, "stopped at the attempt cap");
    }

    #[test]
    fn stop_request_aborts_retrying() {
        // If the source is dropped/stopped mid-retry, bail out promptly.
        let running = AtomicBool::new(false);
        let mut calls = 0u32;
        let track = load_with_retry(0, &running, Duration::ZERO, |_| {
            calls += 1;
            LoadAttempt::Retry
        });
        assert!(track.1.is_empty());
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
}
