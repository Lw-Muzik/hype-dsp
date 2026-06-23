//! Loads impulse-response files (WAV/`.irs`) into interleaved f32 samples for
//! [`hm_dsp::PreparedIr::build`]. Runs OFF the audio thread (file I/O).

use std::path::Path;

use crate::error::AudioError;

/// Decode a WAV/IRS impulse response into `(interleaved_f32, channels, sample_rate)`.
pub fn load_ir_samples(path: &Path) -> Result<(Vec<f32>, usize, f32), AudioError> {
    let reader = hound::WavReader::open(path)
        .map_err(|e| AudioError::Io(format!("open IR {}: {e}", path.display())))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let sample_rate = spec.sample_rate as f32;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| AudioError::Decode(format!("read IR floats: {e}")))?,
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|r| r.map(|v| v as f32 / max))
                .collect::<Result<_, _>>()
                .map_err(|e| AudioError::Decode(format!("read IR ints: {e}")))?
        }
    };
    if samples.is_empty() {
        return Err(AudioError::Decode("IR file is empty".into()));
    }
    Ok((samples, channels, sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_written_wav() {
        // Write a tiny mono 48k WAV to a temp path, then read it back.
        let dir = std::env::temp_dir();
        let path = dir.join("hm_ir_test.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..100 {
            w.write_sample(((i as f32 / 100.0) * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();

        let (samples, ch, sr) = load_ir_samples(&path).unwrap();
        assert_eq!(ch, 1);
        assert_eq!(sr, 48_000.0);
        assert_eq!(samples.len(), 100);
        assert!(samples.iter().all(|&s| (-1.0..=1.0).contains(&s)));
        std::fs::remove_file(&path).ok();
    }
}
