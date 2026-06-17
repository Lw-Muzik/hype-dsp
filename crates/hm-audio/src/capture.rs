//! Capture sources: the driver-free loopback stand-in and the documented
//! virtual-device stub.
//!
//! [`LoopbackCaptureSource`] captures the default **input** device via `cpal`
//! and feeds it through the chain. On Windows this can be the system output
//! (WASAPI loopback); on macOS `cpal` can only capture an input (the mic), so
//! it is an honest dev stand-in — true system-output capture needs the signed
//! virtual driver documented in `docs/audio-driver.md`.
//!
//! [`VirtualDeviceSource`] is that production seam: it reports `Unavailable`
//! until a signed virtual device is installed.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait};
use rtrb::RingBuffer;

use crate::error::AudioError;
use crate::{AudioSource, StreamFormat};

/// Captures the default input device into the chain (driver-free stand-in).
pub struct LoopbackCaptureSource {
    consumer: rtrb::Consumer<f32>,
    running: Arc<AtomicBool>,
    position_frames: Arc<AtomicU64>,
    _thread: JoinHandle<()>,
}

impl LoopbackCaptureSource {
    /// Start capturing. Errors if there is no input device.
    pub fn new(device_rate: u32) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        if host.default_input_device().is_none() {
            return Err(AudioError::Unavailable("no audio input device".into()));
        }
        let capacity = (device_rate.max(8_000) as usize) * 2 * 2;
        let (producer, consumer) = RingBuffer::<f32>::new(capacity);
        let running = Arc::new(AtomicBool::new(true));
        let position_frames = Arc::new(AtomicU64::new(0));

        let thread = {
            let running = running.clone();
            std::thread::Builder::new()
                .name("hm-capture".into())
                .spawn(move || capture_loop(producer, &running))
                .expect("failed to spawn capture thread")
        };

        Ok(Self {
            consumer,
            running,
            position_frames,
            _thread: thread,
        })
    }
}

impl Drop for LoopbackCaptureSource {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl AudioSource for LoopbackCaptureSource {
    fn start(&mut self, _format: StreamFormat) -> Result<(), AudioError> {
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

fn capture_loop(mut producer: rtrb::Producer<f32>, running: &AtomicBool) {
    let host = cpal::default_host();
    let Some(device) = host.default_input_device() else {
        return;
    };
    let Ok(config) = pick_f32_input_config(&device) else {
        return;
    };
    let in_channels = config.channels() as usize;

    let stream = device.build_input_stream::<f32, _, _>(
        config.config(),
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for frame in data.chunks(in_channels.max(1)) {
                let l = frame.first().copied().unwrap_or(0.0);
                let r = frame.get(1).copied().unwrap_or(l);
                // Drop on overflow (consumer slow) to avoid latency buildup.
                let _ = producer.push(l);
                let _ = producer.push(r);
            }
        },
        |_err| {},
        None,
    );

    let Ok(stream) = stream else { return };
    use cpal::traits::StreamTrait;
    if stream.play().is_err() {
        return;
    }
    // Hold the (!Send) stream alive on this thread until cancelled.
    while running.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn pick_f32_input_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig, AudioError> {
    if let Ok(default) = device.default_input_config() {
        if default.sample_format() == cpal::SampleFormat::F32 {
            return Ok(default);
        }
    }
    let configs = device
        .supported_input_configs()
        .map_err(|e| AudioError::Host(e.to_string()))?;
    for range in configs {
        if range.sample_format() == cpal::SampleFormat::F32 {
            return Ok(range.with_max_sample_rate());
        }
    }
    Err(AudioError::UnsupportedFormat(
        "no f32 input configuration".into(),
    ))
}

/// The production virtual-device seam. Reports `Unavailable` until a signed
/// virtual audio device is installed (see `docs/audio-driver.md`).
pub struct VirtualDeviceSource;

impl AudioSource for VirtualDeviceSource {
    fn start(&mut self, _format: StreamFormat) -> Result<(), AudioError> {
        Err(AudioError::Unavailable(
            "virtual audio device not installed (see docs/audio-driver.md)".into(),
        ))
    }
    fn read(&mut self, _out: &mut [f32], _channels: usize) -> usize {
        0
    }
    fn stop(&mut self) {}
    fn is_live(&self) -> bool {
        true
    }
}

/// Whether system-wide capture (the signed virtual device) is available.
/// Always `false` in this build — the architecture is ready; the driver is not.
pub fn virtual_device_available() -> bool {
    false
}
