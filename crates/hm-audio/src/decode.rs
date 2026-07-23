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
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;

use hm_core::TrackMeta;

use crate::error::AudioError;
use crate::meta::extract_metadata;

/// Decoded PCM: interleaved **stereo** `f32` at `sample_rate`, plus the track's
/// now-playing metadata (tags + cover art).
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub meta: TrackMeta,
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

/// Probe an in-memory audio file (e.g. a fully-downloaded stream). `ext_hint`
/// (a file extension like `"mp3"`) helps symphonia pick the demuxer.
fn open_format_bytes(
    bytes: Vec<u8>,
    ext_hint: Option<&str>,
) -> Result<Box<dyn FormatReader>, AudioError> {
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext_hint {
        hint.with_extension(ext);
    }
    symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| AudioError::Decode(e.to_string()))
}

/// Probe a **non-seekable** reader — a network response body being streamed
/// in, wrapped in symphonia's [`ReadOnlySource`] — so it can be decoded on
/// the fly as it downloads, instead of waiting for the whole file. `ext_hint`
/// helps symphonia pick the demuxer, same as [`open_format_bytes`].
///
/// Takes `reader` by a borrow-friendly generic rather than requiring
/// `'static` so a caller (the streamed queue's worker) can pass something
/// that itself borrows stack-local state (a [`TeeReader`](crate::stream_queue) —
/// mirroring every byte to a spool file) and get that borrow back once the
/// returned `Box<dyn FormatReader>` is dropped.
///
/// Most containers demux fine over a forward-only stream (mp3, flac, ogg,
/// wav, and a "faststart" mp4/m4a with its `moov` atom before `mdat`).
/// A container that genuinely needs to seek — an mp4 with `moov` at the
/// tail, not faststart — surfaces as a probe/decode `Err` here; that's a
/// per-track runtime outcome the caller falls back on (decode the fully
/// spooled file whole, which *is* seekable), not a defect in this path.
pub fn open_format_stream<'a, R>(
    reader: R,
    ext_hint: Option<&str>,
) -> Result<Box<dyn FormatReader + 'a>, AudioError>
where
    R: std::io::Read + Send + Sync + 'a,
{
    let source = ReadOnlySource::new(reader);
    let mss = MediaSourceStream::new(Box::new(source), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext_hint {
        hint.with_extension(ext);
    }
    symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| AudioError::Decode(e.to_string()))
}

/// A slice of decoded output, emitted incrementally so a caller can start
/// playback (or otherwise act) before the whole file has been decoded.
pub enum DecodeChunk {
    /// Sent exactly once, right after probing: tags/cover art, the stream's
    /// native sample rate, and — when the container states it (symphonia's
    /// `Track::num_frames`, the same field `probe_track` reads) — the
    /// track's total frame count at that native rate. `None` when the
    /// container doesn't declare it up front (common for a streamed,
    /// non-seekable source); callers use it only as a capacity hint, never
    /// a correctness dependency. Always arrives before any `Pcm`.
    Meta(TrackMeta, u32, Option<u64>),
    /// Source-rate interleaved stereo PCM, in order.
    Pcm(Vec<f32>),
}

/// Decode an audio file to interleaved stereo `f32`.
pub fn decode_file(path: &Path) -> Result<DecodedAudio, AudioError> {
    collect_decoded(open_format(path)?)
}

/// Decode a fully-downloaded audio file held in memory to interleaved stereo
/// `f32`. Used by the streamed crossfade queue (cloud/phone tracks).
pub fn decode_bytes(bytes: Vec<u8>, ext_hint: Option<&str>) -> Result<DecodedAudio, AudioError> {
    collect_decoded(open_format_bytes(bytes, ext_hint)?)
}

/// Drive [`decode_format_chunked`] to completion, collecting `Meta` +
/// concatenated `Pcm` into a single [`DecodedAudio`] — the whole-track
/// behavior `decode_file`/`decode_bytes` have always presented.
fn collect_decoded(format: Box<dyn FormatReader>) -> Result<DecodedAudio, AudioError> {
    let mut meta = None;
    let mut sample_rate = 0;
    let mut samples: Vec<f32> = Vec::new();
    decode_format_chunked(format, 8192, &mut |chunk| {
        match chunk {
            DecodeChunk::Meta(m, r, _frames) => {
                meta = Some(m);
                sample_rate = r;
            }
            DecodeChunk::Pcm(pcm) => samples.extend(pcm),
        }
        true
    })?;

    if samples.is_empty() {
        return Err(AudioError::Decode("file produced no audio".into()));
    }
    Ok(DecodedAudio {
        samples,
        sample_rate,
        meta: meta.unwrap_or_default(),
    })
}

/// Decode an already-probed format reader to interleaved stereo `f32`,
/// streaming results to `sink` as they become available instead of buffering
/// the whole track. Shared by the file and in-memory (stream) decode paths.
///
/// `sink` receives a single [`DecodeChunk::Meta`] right after probing, then
/// zero or more [`DecodeChunk::Pcm`] chunks of at least `min_chunk_frames`
/// frames each (the final chunk may be a smaller partial flush). Returning
/// `false` from `sink` aborts the decode early (teardown) with `Ok(())`.
///
/// `format` carries an explicit lifetime (rather than the `'static` an
/// elided `Box<dyn FormatReader>` would infer) so a reader built over
/// borrowed stack state — [`open_format_stream`]'s streaming case — can be
/// passed here too; every existing (`'static`) caller still coerces in
/// unchanged.
pub fn decode_format_chunked<'a>(
    mut format: Box<dyn FormatReader + 'a>,
    min_chunk_frames: usize,
    sink: &mut dyn FnMut(DecodeChunk) -> bool,
) -> Result<(), AudioError> {
    // Tags + cover are available right after probing (front-loaded ID3/Vorbis).
    let meta = extract_metadata(&mut *format);
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| AudioError::Decode("no audio track in file".into()))?;
    let track_id = track.id;
    let num_frames = track.num_frames;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|c| c.audio())
        .cloned()
        .ok_or_else(|| AudioError::Decode("missing audio codec parameters".into()))?;
    let sample_rate = params.sample_rate.unwrap_or(44_100);

    if !sink(DecodeChunk::Meta(meta, sample_rate, num_frames)) {
        return Ok(());
    }

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| AudioError::Decode(e.to_string()))?;

    let mut pending: Vec<f32> = Vec::new();
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
                append_stereo(&mut pending, &scratch, channels);
                if pending.len() / 2 >= min_chunk_frames
                    && !sink(DecodeChunk::Pcm(std::mem::take(&mut pending)))
                {
                    return Ok(());
                }
            }
            Err(SymError::DecodeError(_)) => continue,
            Err(ref e) if is_eof(e) => break,
            Err(e) => return Err(AudioError::Decode(e.to_string())),
        }
    }

    if !pending.is_empty() {
        // Terminal statement: the decode loop is already over, so there's
        // nothing left for the sink to abort by returning `false` — its
        // return value is deliberately unchecked here.
        sink(DecodeChunk::Pcm(pending));
    }
    Ok(())
}

/// Read a file's text tags (title/artist/album/genre) without decoding audio,
/// for the library scan. Cheap enough to run over a whole folder. Returns
/// default (all `None`) if the file can't be probed.
pub fn probe_tags(path: &Path) -> crate::meta::TrackTags {
    match open_format(path) {
        Ok(mut format) => crate::meta::extract_tags(&mut *format),
        Err(_) => crate::meta::TrackTags::default(),
    }
}

/// Read a file's tags **and** duration in a single open, for the library scan —
/// half the I/O of calling `probe_tags` + `probe_duration` separately. This
/// matters when importing tens of thousands of files.
pub fn probe_track(path: &Path) -> (crate::meta::TrackTags, Option<f64>) {
    let Ok(mut format) = open_format(path) else {
        return (crate::meta::TrackTags::default(), None);
    };
    let tags = crate::meta::extract_tags(&mut *format);
    let duration = format
        .default_track(TrackType::Audio)
        .and_then(|t| {
            let params = t.codec_params.as_ref()?.audio()?;
            let rate = params.sample_rate? as f64;
            let frames = t.num_frames? as f64;
            (rate > 0.0).then_some(frames / rate)
        });
    (tags, duration)
}

/// Read a file's embedded front-cover art as a `data:` URI, or `None` if it has
/// none / can't be probed. Used to lazily fill library artwork on demand.
pub fn probe_artwork(path: &Path) -> Option<String> {
    let mut format = open_format(path).ok()?;
    extract_metadata(&mut *format).cover
}

/// Read a file's embedded lyrics (plain or LRC), or `None`. Used by the lyrics
/// resolution chain before falling back to an online lookup.
pub fn probe_lyrics(path: &Path) -> Option<String> {
    let mut format = open_format(path).ok()?;
    crate::meta::extract_lyrics(&mut *format)
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

/// Chunk-at-a-time [`resample_stereo`]: same arithmetic, same output,
/// bit-for-bit, no matter where the input is split.
///
/// Works on GLOBAL indices — the next output frame `i` and the total source
/// frames seen — so a chunk boundary never becomes an interpolation boundary.
/// Emits output frame `i` only once source frame `floor(i/ratio) + 1` exists
/// (interpolation needs its right-hand neighbour); `finish()` flushes the
/// tail, where the reference clamps the neighbour to the last frame.
///
/// The bit-identity guarantee hinges on one bound: `emit`'s mid-stream cap
/// of `round(avail * ratio)` output frames, which — because `round(N *
/// ratio)` is non-decreasing in `N` — never emits a frame the one-shot
/// reference (bounded by the *final* frame count) wouldn't also emit.
pub struct StreamResampler {
    src_rate: u32,
    dst_rate: u32,
    /// Interleaved source frames not yet consumed (the pending tail).
    buf: Vec<f32>,
    /// Source frames dropped from the front of `buf` so far (global offset).
    consumed: usize,
    /// Next output frame index (global).
    next_out: usize,
}

impl StreamResampler {
    pub fn new(src_rate: u32, dst_rate: u32) -> Self {
        Self {
            src_rate,
            dst_rate,
            buf: Vec::new(),
            consumed: 0,
            next_out: 0,
        }
    }

    /// Feed source-rate frames; get back every output frame now computable.
    pub fn push(&mut self, chunk: &[f32]) -> Vec<f32> {
        if self.src_rate == self.dst_rate {
            return chunk.to_vec();
        }
        self.buf.extend_from_slice(chunk);
        self.emit(false)
    }

    /// The stream is over: emit the tail with the reference's end-clamping.
    pub fn finish(&mut self) -> Vec<f32> {
        if self.src_rate == self.dst_rate {
            return Vec::new();
        }
        self.emit(true)
    }

    fn emit(&mut self, at_end: bool) -> Vec<f32> {
        let ratio = self.dst_rate as f64 / self.src_rate as f64;
        let avail = self.consumed + self.buf.len() / 2; // global frames present
        // The reference bounds output by `round(N_final * ratio)`, computed
        // against the *final* total frame count `N_final`. Mid-stream we
        // only know `avail` (frames seen so far, `avail <= N_final`), but
        // `round(N * ratio)` is non-decreasing in `N`, so bounding eagerly
        // by `round(avail * ratio)` is always <= the true final bound —
        // never emits a frame the reference wouldn't also emit, for ANY
        // ratio (this is what breaks for ratio < 0.5 without the bound: a
        // neighbour can exist for an output index the reference will never
        // reach). At `finish()`, `avail == N_final`, so this becomes exactly
        // the reference's own `out_frames`.
        let out_end = ((avail as f64) * ratio).round() as usize;
        let last = avail.saturating_sub(1);
        let mut out = Vec::new();
        while self.next_out < out_end {
            let src_pos = self.next_out as f64 / ratio;
            let idx = src_pos.floor() as usize;
            // Mid-stream we must not clamp: wait for the neighbour instead.
            if !at_end && idx + 1 >= avail {
                break;
            }
            let frac = (src_pos - idx as f64) as f32;
            let i0 = idx.min(last);
            let i1 = (idx + 1).min(last);
            let (Some(a0), Some(a1)) = (self.local(i0), self.local(i1)) else {
                break;
            };
            for ch in 0..2 {
                let a = a0[ch];
                let b = a1[ch];
                out.push(a + (b - a) * frac);
            }
            self.next_out += 1;
        }
        // Drop source frames no future output frame will read. The earliest
        // future read is floor(next_out/ratio) — that frame itself becomes
        // the left neighbour of the next emission, so it must be kept.
        let keep_from = ((self.next_out as f64 / ratio).floor() as usize).min(avail);
        if keep_from > self.consumed {
            let drop_frames = keep_from - self.consumed;
            self.buf.drain(0..(drop_frames * 2).min(self.buf.len()));
            self.consumed = keep_from;
        }
        out
    }

    /// Global frame `i` out of the retained buffer, if still present.
    fn local(&self, i: usize) -> Option<[f32; 2]> {
        let rel = i.checked_sub(self.consumed)?;
        let base = rel * 2;
        (base + 1 < self.buf.len()).then(|| [self.buf[base], self.buf[base + 1]])
    }
}

#[cfg(test)]
pub(crate) mod tests {
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

    /// Chunked resampling must be indistinguishable from one-shot: same
    /// arithmetic, same output, regardless of where the input was split.
    fn assert_chunked_matches(input: &[f32], src: u32, dst: u32, splits: &[usize]) {
        let reference = resample_stereo(input, src, dst);
        let mut r = StreamResampler::new(src, dst);
        let mut out = Vec::new();
        let mut at = 0;
        for &n in splits {
            let end = (at + n * 2).min(input.len());
            out.extend(r.push(&input[at..end]));
            at = end;
        }
        out.extend(r.push(&input[at..]));
        out.extend(r.finish());
        assert_eq!(out.len(), reference.len(), "same frame count");
        assert!(
            out.iter().zip(&reference).all(|(a, b)| a == b),
            "chunked output must be bit-identical to the one-shot reference"
        );
    }

    fn ramp(frames: usize) -> Vec<f32> {
        (0..frames * 2).map(|i| i as f32 / 100.0).collect()
    }

    #[test]
    fn chunked_resample_matches_one_shot_upsampling() {
        assert_chunked_matches(&ramp(1000), 44_100, 48_000, &[1, 7, 100, 250]);
    }

    #[test]
    fn chunked_resample_matches_one_shot_downsampling() {
        assert_chunked_matches(&ramp(1000), 48_000, 44_100, &[3, 500, 1]);
    }

    #[test]
    fn chunked_resample_is_identity_at_same_rate() {
        assert_chunked_matches(&ramp(64), 48_000, 48_000, &[10, 10]);
    }

    #[test]
    fn one_frame_chunks_still_match() {
        assert_chunked_matches(&ramp(48), 44_100, 48_000, &[1; 40]);
    }

    #[test]
    fn chunked_resample_handles_empty_input() {
        assert_chunked_matches(&[], 44_100, 48_000, &[]);
    }

    #[test]
    fn chunked_resample_handles_zero_length_pushes() {
        // Zero-length `push` calls (no frames available yet, or a decoder
        // hiccup) must be harmless no-ops, not corrupt the stream.
        assert_chunked_matches(&ramp(200), 44_100, 48_000, &[0, 5, 0, 0, 50, 0]);
    }

    #[test]
    fn chunked_resample_single_frame_input() {
        assert_chunked_matches(&ramp(1), 44_100, 48_000, &[1]);
    }

    // Regression coverage for a critical bug found in review: for ratio <
    // 0.5 (heavy downsampling), a right-hand interpolation neighbour can
    // exist mid-stream for an output index the one-shot reference — bounded
    // by `round(N_final * ratio)` — will NEVER emit. Gating solely on
    // "neighbour exists" (no `avail`-bound) over-emits in that regime;
    // `finish()` cannot retract already-returned frames. These ratios are
    // real for this pipeline once hi-res (96k/192k) source files are decoded
    // down to a 44.1k/48k device rate.
    #[test]
    fn chunked_resample_matches_one_shot_heavy_downsampling() {
        assert_chunked_matches(&ramp(1200), 96_000, 44_100, &[1, 1, 1, 5, 250, 1, 1, 900]);
    }

    #[test]
    fn chunked_resample_matches_one_shot_extreme_downsampling() {
        assert_chunked_matches(&ramp(1200), 192_000, 44_100, &[1, 3, 400, 1, 1, 1]);
    }

    #[test]
    fn chunked_resample_matches_at_the_half_ratio_boundary() {
        // ratio == 0.5 exactly: the boundary between the regime the original
        // neighbour-only gate handled correctly and the regime it broke.
        assert_chunked_matches(&ramp(1000), 96_000, 48_000, &[1, 1, 1, 100, 400, 1]);
    }

    /// A minimal 16-bit stereo PCM WAV: enough for symphonia to decode.
    /// `pub(crate)`: reused by `stream_queue`'s tests to build a real,
    /// probeable body for its local-TCP-harness event-sequence tests.
    pub(crate) fn tiny_wav(frames: usize, rate: u32) -> Vec<u8> {
        let data_len = (frames * 4) as u32;
        let mut w = Vec::new();
        w.extend(b"RIFF");
        w.extend((36 + data_len).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes()); // PCM
        w.extend(2u16.to_le_bytes()); // stereo
        w.extend(rate.to_le_bytes());
        w.extend((rate * 4).to_le_bytes()); // byte rate
        w.extend(4u16.to_le_bytes()); // block align
        w.extend(16u16.to_le_bytes()); // bits
        w.extend(b"data");
        w.extend(data_len.to_le_bytes());
        for i in 0..frames {
            let v = ((i % 97) as i16).wrapping_mul(199);
            w.extend(v.to_le_bytes()); // L
            w.extend((-v).to_le_bytes()); // R
        }
        w
    }

    #[test]
    fn chunked_decode_equals_whole_decode() {
        let wav = tiny_wav(10_000, 44_100);
        let whole = decode_bytes(wav.clone(), Some("wav")).expect("whole decode");

        let format = open_format_bytes(wav, Some("wav")).expect("probe");
        let mut meta = None;
        let mut rate = 0;
        let mut samples = Vec::new();
        decode_format_chunked(format, 1024, &mut |c| {
            match c {
                DecodeChunk::Meta(m, r, _frames) => {
                    meta = Some(m);
                    rate = r;
                }
                DecodeChunk::Pcm(pcm) => samples.extend(pcm),
            }
            true
        })
        .expect("chunked decode");

        assert_eq!(rate, whole.sample_rate);
        assert_eq!(samples.len(), whole.samples.len());
        assert!(samples.iter().zip(&whole.samples).all(|(a, b)| a == b), "bit-identical PCM");
    }

    #[test]
    fn chunked_decode_flushes_in_bounded_chunks() {
        let wav = tiny_wav(10_000, 44_100);
        let format = open_format_bytes(wav, Some("wav")).expect("probe");
        let mut sizes = Vec::new();
        decode_format_chunked(format, 1024, &mut |c| {
            if let DecodeChunk::Pcm(p) = c {
                sizes.push(p.len() / 2);
            }
            true
        })
        .unwrap();
        assert!(sizes.len() > 1, "10k frames at min 1024 must arrive in several chunks");
        // Every flush waited for the minimum except the final partial one.
        assert!(sizes[..sizes.len() - 1].iter().all(|&s| s >= 1024));
    }

    #[test]
    fn a_false_from_the_sink_aborts_the_decode() {
        let wav = tiny_wav(50_000, 44_100);
        let format = open_format_bytes(wav, Some("wav")).expect("probe");
        let mut pcm_calls = 0;
        let _ = decode_format_chunked(format, 256, &mut |c| {
            if matches!(c, DecodeChunk::Pcm(_)) {
                pcm_calls += 1;
            }
            pcm_calls < 2 // stop after the second PCM chunk
        });
        assert_eq!(pcm_calls, 2, "the decode must stop when the sink says stop");
    }

    #[test]
    fn chunked_resample_length_sweep_matches_one_shot() {
        // Sweep every frame count 1..=300 at a heavy-downsampling ratio,
        // single monolithic push (chunking isn't the variable here — the
        // off-by-one is in the emission bound itself, so this must fail on
        // the buggy version even with a single `push` + `finish`).
        for n in 1..=300usize {
            let input = ramp(n);
            let reference = resample_stereo(&input, 96_000, 44_100);
            let mut r = StreamResampler::new(96_000, 44_100);
            let mut out = r.push(&input);
            out.extend(r.finish());
            assert_eq!(out.len(), reference.len(), "frame count mismatch at N={n}");
            assert!(
                out.iter().zip(&reference).all(|(a, b)| a == b),
                "content mismatch at N={n}"
            );
        }
    }
}
