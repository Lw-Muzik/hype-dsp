//! File decoding and sample-rate conversion.
//!
//! Phase 2 decodes **WAV** (lossless, real — no stubbing) via `hound`, which is
//! enough to route a real file through the DSP chain and validate the audio
//! path end to end. Compressed formats (mp3/flac/aac/ogg) arrive with the
//! symphonia integration in Phase 5.

use std::path::Path;

use crate::error::AudioError;

/// Decoded PCM: interleaved **stereo** `f32` at `sample_rate`.
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// Decode an audio file to interleaved stereo `f32`.
pub fn decode_file(path: &Path) -> Result<DecodedAudio, AudioError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);

    match ext.as_deref() {
        Some("wav") => decode_wav(path),
        other => Err(AudioError::UnsupportedFormat(format!(
            "Phase 2 plays .wav files; {} support arrives in Phase 5",
            other.unwrap_or("this file's")
        ))),
    }
}

fn decode_wav(path: &Path) -> Result<DecodedAudio, AudioError> {
    let mut reader = hound::WavReader::open(path).map_err(|e| AudioError::Io(e.to_string()))?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    if channels == 0 {
        return Err(AudioError::Decode("file reports zero channels".into()));
    }

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.unwrap_or(0) as f32 * scale)
                .collect()
        }
    };

    Ok(DecodedAudio {
        samples: to_stereo(&interleaved, channels),
        sample_rate: spec.sample_rate,
    })
}

/// Fold an interleaved buffer of `channels` channels down/up to stereo.
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

/// Linearly resample interleaved stereo from `src_rate` to `dst_rate`.
///
/// Linear interpolation is adequate for Phase 2's "hear the DSP" goal; a
/// higher-quality polyphase resampler can replace this later without changing
/// callers.
pub fn resample_stereo(samples: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    let frames = samples.len() / 2;
    if src_rate == dst_rate || frames == 0 {
        return samples.to_vec();
    }
    let ratio = dst_rate as f64 / src_rate as f64;
    let out_frames = ((frames as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let src_pos = i as f64 / ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let i0 = idx.min(frames - 1);
        let i1 = (idx + 1).min(frames - 1);
        for ch in 0..2 {
            let a = samples[i0 * 2 + ch];
            let b = samples[i1 * 2 + ch];
            out.push(a + (b - a) * frac);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_is_identity_at_same_rate() {
        let input = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(resample_stereo(&input, 48_000, 48_000), input);
    }

    #[test]
    fn resample_doubling_rate_doubles_frames() {
        // 4 stereo frames at 24k -> ~8 frames at 48k.
        let input = vec![0.0; 8];
        let out = resample_stereo(&input, 24_000, 48_000);
        assert_eq!(out.len(), 16);
    }

    #[test]
    fn decode_roundtrips_a_generated_wav() {
        let dir = std::env::temp_dir();
        let path = dir.join("hm_audio_decode_test.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..100 {
            writer.write_sample(16_384i16).unwrap(); // ~0.5 L
            writer.write_sample(-16_384i16).unwrap(); // ~-0.5 R
        }
        writer.finalize().unwrap();

        let decoded = decode_file(&path).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.samples.len(), 200);
        assert!((decoded.samples[0] - 0.5).abs() < 0.01);
        assert!((decoded.samples[1] + 0.5).abs() < 0.01);
        let _ = std::fs::remove_file(&path);
    }
}
