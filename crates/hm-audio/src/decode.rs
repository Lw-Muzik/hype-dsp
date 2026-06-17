//! File decoding and sample-rate conversion.
//!
//! Decodes mp3/flac/aac/wav/ogg/vorbis/mp4 via **symphonia** to interleaved
//! stereo `f32`, then resamples (linearly) to the device rate off the audio
//! thread. The audio thread only ever copies from the decoded buffer.

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::error::AudioError;

/// Decoded PCM: interleaved **stereo** `f32` at `sample_rate`.
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

fn open_format(path: &Path) -> Result<Box<dyn FormatReader>, AudioError> {
    let file = File::open(path).map_err(|e| AudioError::Io(e.to_string()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| AudioError::Decode(e.to_string()))
}

fn is_eof(e: &SymError) -> bool {
    matches!(e, SymError::IoError(io) if io.kind() == std::io::ErrorKind::UnexpectedEof)
}

/// Decode an audio file to interleaved stereo `f32`.
pub fn decode_file(path: &Path) -> Result<DecodedAudio, AudioError> {
    let mut format = open_format(path)?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| AudioError::Decode("no audio track in file".into()))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|c| c.audio())
        .cloned()
        .ok_or_else(|| AudioError::Decode("missing audio codec parameters".into()))?;
    let sample_rate = params.sample_rate.unwrap_or(44_100);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| AudioError::Decode(e.to_string()))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(ref e) if is_eof(e) => break,
            Err(e) => return Err(AudioError::Decode(e.to_string())),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                let channels = audio.spec().channels().count().max(1);
                scratch.clear();
                audio.copy_to_vec_interleaved::<f32>(&mut scratch);
                append_stereo(&mut samples, &scratch, channels);
            }
            Err(SymError::DecodeError(_)) => continue,
            Err(ref e) if is_eof(e) => break,
            Err(e) => return Err(AudioError::Decode(e.to_string())),
        }
    }

    if samples.is_empty() {
        return Err(AudioError::Decode("file produced no audio".into()));
    }
    Ok(DecodedAudio {
        samples,
        sample_rate,
    })
}

/// Probe a file's duration in seconds without fully decoding it (for the
/// library scan). Returns `None` if unknown.
pub fn probe_duration(path: &Path) -> Option<f64> {
    let format = open_format(path).ok()?;
    let track = format.default_track(TrackType::Audio)?;
    let params = track.codec_params.as_ref()?.audio()?;
    let rate = params.sample_rate? as f64;
    let frames = track.num_frames? as f64;
    if rate > 0.0 {
        Some(frames / rate)
    } else {
        None
    }
}

fn append_stereo(out: &mut Vec<f32>, interleaved: &[f32], channels: usize) {
    if channels == 0 {
        return;
    }
    let frames = interleaved.len() / channels;
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
}

/// Linearly resample interleaved stereo from `src_rate` to `dst_rate`.
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
        let input = vec![0.0; 8];
        let out = resample_stereo(&input, 24_000, 48_000);
        assert_eq!(out.len(), 16);
    }

    #[test]
    fn decodes_a_generated_wav() {
        let path = std::env::temp_dir().join("hm_audio_decode_test.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..200 {
            writer.write_sample(16_384i16).unwrap();
            writer.write_sample(-16_384i16).unwrap();
        }
        writer.finalize().unwrap();

        let decoded = decode_file(&path).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        assert!(decoded.samples.len() >= 400);
        assert!((decoded.samples[0] - 0.5).abs() < 0.02);
        assert!((decoded.samples[1] + 0.5).abs() < 0.02);
        assert!(probe_duration(&path).unwrap() > 0.0);
        let _ = std::fs::remove_file(&path);
    }
}
