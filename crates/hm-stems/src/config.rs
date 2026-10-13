//! Spleeter's training-time STFT contract and the chunk geometry. A direct
//! port of the mobile engine's `stem_config.h` (the canonical copy lives in the
//! `hype` repo under `android/app/src/main/cpp/stems/`) — change a number here
//! and the parity test against that C++ code fails.

/// Spleeter works at 44.1 kHz stereo; every buffer in this crate is at it.
pub const MODEL_RATE: u32 = 44_100;
pub const NFFT: usize = 4096;
pub const HOP: usize = 1024;
/// Complex bins per frame (`NFFT / 2 + 1`).
pub const SPEC_BINS: usize = NFFT / 2 + 1;
/// The nets see only the lowest 1024 bins (~11 kHz); the rest is extended.
pub const NET_BINS: usize = 1024;
/// Frames per net run (fixed by the exported graph).
pub const SPLIT_FRAMES: usize = 512;
/// Front padding: frame `f` of a chunk starts at `f * HOP - PAD`.
pub const PAD: usize = NFFT - HOP;

/// Consecutive chunks overlap by 4 frames (every sample is covered by
/// `NFFT / HOP` = 4 frames, so a chunk finalises only samples whose 4 frames
/// all lie inside it).
pub const STEP_FRAMES: usize = SPLIT_FRAMES - 4;
/// Output samples one chunk owns (~11.8 s).
pub const STEP: usize = STEP_FRAMES * HOP;
/// Exact output on both sides of an owned range.
pub const MARGIN: usize = 512;
/// Input samples one chunk reads.
pub const INPUT_SPAN: usize = SPLIT_FRAMES * HOP + PAD;

/// The four Spleeter nets, in the order the mobile engine runs them.
pub const NET_COUNT: usize = 4;
pub const NET_VOCALS: usize = 0;
pub const NET_DRUMS: usize = 1;
pub const NET_BASS: usize = 2;
pub const NET_OTHER: usize = 3;

/// First sample (model domain) of grid chunk `c`'s owned range.
pub fn owned_start(c: usize) -> usize {
    if c == 0 {
        0
    } else {
        c * STEP + MARGIN
    }
}

/// One past the last sample of grid chunk `c`'s owned range.
pub fn owned_end(c: usize) -> usize {
    (c + 1) * STEP + MARGIN
}

/// The grid chunk that owns `frame`.
pub fn chunk_containing(frame: usize) -> usize {
    if frame < owned_end(0) {
        0
    } else {
        (frame - MARGIN) / STEP
    }
}

/// How many grid chunks cover `frames` samples.
pub fn chunk_count(frames: usize) -> usize {
    if frames == 0 {
        0
    } else {
        chunk_containing(frames - 1) + 1
    }
}

/// Where chunk `c` starts reading input (negative = before the track: zeros).
pub fn span_start(c: usize) -> i64 {
    if c == 0 {
        -(PAD as i64)
    } else {
        owned_start(c) as i64 - MARGIN as i64 - PAD as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_constants_match_the_mobile_header() {
        assert_eq!(SPEC_BINS, 2049);
        assert_eq!(PAD, 3072);
        assert_eq!(STEP, 520_192);
        assert_eq!(INPUT_SPAN, 527_360);
    }

    #[test]
    fn grid_tiles_the_timeline_without_gaps() {
        for c in 0..20 {
            assert_eq!(owned_end(c), owned_start(c + 1));
        }
        for frame in [0, 1, 520_703, 520_704, 520_705, 3_000_000, 10_000_000] {
            let c = chunk_containing(frame);
            assert!(owned_start(c) <= frame && frame < owned_end(c), "frame {frame} chunk {c}");
        }
    }

    #[test]
    fn owned_range_lies_inside_the_exact_region() {
        // The exact region of a chunk is the samples all 4 of whose frames are in it.
        for c in 0..5 {
            let span = span_start(c);
            let exact_lo = span + PAD as i64;
            let exact_hi = span + (SPLIT_FRAMES * HOP) as i64;
            assert!(exact_lo <= owned_start(c) as i64, "chunk {c} start");
            assert!(owned_end(c) as i64 <= exact_hi, "chunk {c} end");
        }
    }

    #[test]
    fn chunk_count_covers_the_track() {
        assert_eq!(chunk_count(0), 0);
        assert_eq!(chunk_count(1), 1);
        assert_eq!(chunk_count(owned_end(0)), 1);
        assert_eq!(chunk_count(owned_end(0) + 1), 2);
        let n = chunk_count(44_100 * 240);
        assert!(owned_end(n - 1) >= 44_100 * 240 && owned_end(n - 2) < 44_100 * 240);
    }
}
