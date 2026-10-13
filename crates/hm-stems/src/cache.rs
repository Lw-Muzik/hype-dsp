//! On-disk cache of a finished separation: vocals and drums as 16-bit stereo
//! WAVs (instruments are derived, so two files, not four). The mobile engine
//! stores the same pair as int16 at half scale, which keeps peaks that overshoot
//! full scale representable.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::{StemError, MODEL_RATE};

/// Full-scale headroom: samples are stored as `x * SCALE * 32768`.
const SCALE: f32 = 0.5;
const VOCALS: &str = "vocals.wav";
const DRUMS: &str = "drums.wav";

/// A stable key for a source file: name + size + mtime, so edits re-separate
/// but identical files are reused.
pub fn file_key(input: &Path) -> String {
    let name = input.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
    let (size, mtime) = std::fs::metadata(input)
        .map(|m| {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (m.len(), mtime)
        })
        .unwrap_or((0, 0));
    format!("{}-{size}-{mtime}", sanitize(name))
}

/// A cache key for a non-file source (e.g. a YouTube Music video id).
pub fn id_key(namespace: &str, id: &str) -> String {
    format!("{}-{}", sanitize(namespace), sanitize(id))
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(48)
        .collect()
}

pub fn slot(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join(key)
}

fn write_one(path: &Path, samples: &[f32]) -> Result<(), StemError> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: MODEL_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(io)?;
    for &s in samples {
        let q = (s * SCALE * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
        w.write_sample(q).map_err(io)?;
    }
    w.finalize().map_err(io)
}

fn read_one(path: &Path) -> Option<Vec<f32>> {
    let mut r = hound::WavReader::open(path).ok()?;
    if r.spec().bits_per_sample != 16 || r.spec().channels != 2 {
        return None;
    }
    let k = 1.0 / (SCALE * 32768.0);
    r.samples::<i16>().map(|s| s.ok().map(|v| f32::from(v) * k)).collect()
}

/// Cache a finished separation. Written to a temp dir and renamed, so a crash
/// never leaves a half-written entry that reads back as valid.
pub fn write(cache_dir: &Path, key: &str, vocals: &[f32], drums: &[f32]) -> Result<(), StemError> {
    let dir = slot(cache_dir, key);
    let tmp = cache_dir.join(format!("{key}.tmp"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(io)?;
    write_one(&tmp.join(VOCALS), vocals)?;
    write_one(&tmp.join(DRUMS), drums)?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&tmp, &dir).map_err(io)
}

/// Read a cached separation if it matches `frames` (a re-decode that came out a
/// different length can't use it).
pub fn read(cache_dir: &Path, key: &str, frames: usize) -> Option<(Vec<f32>, Vec<f32>)> {
    let dir = slot(cache_dir, key);
    let v = read_one(&dir.join(VOCALS))?;
    let d = read_one(&dir.join(DRUMS))?;
    (v.len() == frames * 2 && d.len() == frames * 2).then_some((v, d))
}

pub fn exists(cache_dir: &Path, key: &str) -> bool {
    let dir = slot(cache_dir, key);
    dir.join(VOCALS).is_file() && dir.join(DRUMS).is_file()
}

/// Delete the oldest entries until the cache is under `max_bytes`.
pub fn prune(cache_dir: &Path, max_bytes: u64) {
    let Ok(rd) = std::fs::read_dir(cache_dir) else { return };
    let mut entries: Vec<(SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let size = std::fs::read_dir(e.path())
                .map(|d| d.flatten().filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum())
                .unwrap_or(0);
            let modified = e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            (modified, size, e.path())
        })
        .collect();
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    entries.sort_by_key(|e| e.0);
    for (_, size, path) in entries {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_dir_all(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

fn io(e: impl std::fmt::Display) -> StemError {
    StemError::Io(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hm_stems_cache_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn key_is_stable_and_sanitized() {
        let k = file_key(Path::new("/music/My Song (2024)!.flac"));
        assert!(k.starts_with("My_Song__2024__"));
        assert!(!k.contains('/') && !k.contains(' '));
        assert_eq!(id_key("yt", "a/b c"), "yt-a_b_c");
    }

    #[test]
    fn round_trips_with_headroom_and_checks_length() {
        let dir = tmp("rt");
        let v: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.01).sin() * 1.8).collect(); // beyond full scale
        let d: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.02).cos() * 0.3).collect();
        write(&dir, "k", &v, &d).unwrap();
        assert!(exists(&dir, "k"));
        let (v2, d2) = read(&dir, "k", 500).unwrap();
        for (a, b) in v.iter().zip(&v2).chain(d.iter().zip(&d2)) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
        }
        assert!(read(&dir, "k", 499).is_none(), "length mismatch is a miss");
        assert!(read(&dir, "missing", 500).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_drops_the_oldest_first() {
        let dir = tmp("prune");
        for k in ["old", "new"] {
            write(&dir, k, &vec![0.1; 20_000], &vec![0.1; 20_000]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let one = std::fs::read_dir(dir.join("new")).unwrap().flatten().map(|f| f.metadata().unwrap().len()).sum::<u64>();
        prune(&dir, one + 10);
        assert!(!exists(&dir, "old") && exists(&dir, "new"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
