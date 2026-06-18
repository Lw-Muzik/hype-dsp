//! Internet radio streaming source.
//!
//! A dedicated thread does a blocking HTTP GET, decodes the stream with
//! symphonia (over a non-seekable [`ReadOnlySource`]), resamples to the device
//! rate, and pushes interleaved-stereo samples into a lock-free SPSC ring. The
//! audio callback pulls from the ring via [`AudioSource::read`], emitting
//! silence while buffering. It is a **live** source, so underflow never ends
//! playback.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

/// A radio stream rendered as an [`AudioSource`].
pub struct RadioStreamSource {
    consumer: rtrb::Consumer<f32>,
    running: Arc<AtomicBool>,
    position_frames: Arc<AtomicU64>,
    _thread: JoinHandle<()>,
}

impl RadioStreamSource {
    /// Start streaming `url`, producing stereo at `device_rate`.
    pub fn new(url: String, device_rate: u32) -> Self {
        Self::with_headers(url, Vec::new(), device_rate)
    }

    /// Start streaming `url` with extra HTTP request headers (e.g. an
    /// `Authorization: Bearer …` for a Google Drive `alt=media` URL).
    pub fn with_headers(url: String, headers: Vec<(String, String)>, device_rate: u32) -> Self {
        // ~8 seconds of stereo headroom.
        let capacity = (device_rate.max(8_000) as usize) * 2 * 8;
        let (producer, consumer) = RingBuffer::<f32>::new(capacity);
        let running = Arc::new(AtomicBool::new(true));
        let position_frames = Arc::new(AtomicU64::new(0));

        let thread = {
            let running = running.clone();
            std::thread::Builder::new()
                .name("hm-radio-decode".into())
                .spawn(move || decode_stream(&url, &headers, device_rate, producer, &running))
                .expect("failed to spawn radio decode thread")
        };

        Self {
            consumer,
            running,
            position_frames,
            _thread: thread,
        }
    }
}

impl Drop for RadioStreamSource {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
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
        let mut produced = 0;
        for f in 0..frames {
            let base = f * channels;
            if self.consumer.slots() >= 2 {
                let l = self.consumer.pop().unwrap_or(0.0);
                let r = self.consumer.pop().unwrap_or(0.0);
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
                // Buffering / underflow: emit silence (live source keeps going).
                for ch in out.iter_mut().take(base + channels).skip(base) {
                    *ch = 0.0;
                }
            }
        }
        self.position_frames
            .fetch_add(produced as u64, Ordering::Relaxed);
        produced
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }

    fn position(&self) -> usize {
        self.position_frames.load(Ordering::Relaxed) as usize
    }

    fn is_live(&self) -> bool {
        true
    }
}

fn decode_stream(
    url: &str,
    headers: &[(String, String)],
    device_rate: u32,
    mut producer: Producer<f32>,
    running: &AtomicBool,
) {
    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(12))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let response = match req.send() {
        Ok(r) if r.status().is_success() => r,
        _ => return,
    };

    let mss = MediaSourceStream::new(Box::new(ReadOnlySource::new(response)), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = url.rsplit('.').next() {
        if matches!(ext, "mp3" | "aac" | "ogg" | "flac" | "m4a") {
            hint.with_extension(ext);
        }
    }

    let Ok(mut format) = symphonia::default::get_probe().probe(
        &hint,
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) else {
        return;
    };
    let Some(track) = format.default_track(TrackType::Audio) else {
        return;
    };
    let track_id = track.id;
    let Some(params) = track.codec_params.as_ref().and_then(|c| c.audio()).cloned() else {
        return;
    };
    let stream_rate = params.sample_rate.unwrap_or(44_100);
    let Ok(mut decoder) = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
    else {
        return;
    };

    let mut scratch: Vec<f32> = Vec::new();
    while running.load(Ordering::Relaxed) {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            _ => break,
        };
        if packet.track_id != track_id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(a) => a,
            Err(SymError::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let ch = audio.spec().channels().count().max(1);
        scratch.clear();
        audio.copy_to_vec_interleaved::<f32>(&mut scratch);

        let stereo = to_stereo(&scratch, ch);
        let resampled = resample_stereo(&stereo, stream_rate, device_rate);
        if !push_all(&mut producer, &resampled, running) {
            break;
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

/// Push all samples, backpressuring (sleep) when the ring is full. Returns
/// `false` if streaming was cancelled.
fn push_all(producer: &mut Producer<f32>, samples: &[f32], running: &AtomicBool) -> bool {
    for &s in samples {
        loop {
            if !running.load(Ordering::Relaxed) {
                return false;
            }
            match producer.push(s) {
                Ok(()) => break,
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
    true
}
