//! Tests that need the real Spleeter nets and an ONNX Runtime library. All are
//! `#[ignore]`d; run them explicitly:
//!
//! ```text
//! HM_STEMS_MODEL_DIR=<dir with vocals/drums/bass/other.onnx> \
//! ORT_DYLIB_PATH=<libonnxruntime.dylib> \
//! HM_STEMS_PARITY_INPUT=<clip.f32> HM_STEMS_PARITY_DIR=<parity/run.sh output> \
//! cargo test -p hm-stems --release -- --ignored --nocapture
//! ```
//!
//! `parity/run.sh` builds the MOBILE C++ engine's separation code and writes the
//! reference vocals/drums; `parity` compares the Rust port against them.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use hm_stems::chunk::{separate_chunk, ChunkScratch};
use hm_stems::config::{owned_end, owned_start};
use hm_stems::{OrtNets, StemStore};

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("set {name}")))
}

fn read_f32(path: &std::path::Path) -> Vec<f32> {
    std::fs::read(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// Signal-to-distortion ratio of `x` against reference `r`, in dB.
fn sdr_db(r: &[f32], x: &[f32]) -> f64 {
    let sig: f64 = r.iter().map(|&v| f64::from(v).powi(2)).sum();
    let err: f64 = r.iter().zip(x).map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2)).sum();
    10.0 * (sig / err.max(1e-30)).log10()
}

fn max_abs(r: &[f32], x: &[f32]) -> f32 {
    r.iter().zip(x).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max)
}

#[test]
#[ignore = "needs the nets, ORT, and the C++ reference output"]
fn parity_with_the_mobile_cpp_engine() {
    let mix = read_f32(&env_path("HM_STEMS_PARITY_INPUT"));
    let dir = env_path("HM_STEMS_PARITY_DIR");
    let mut nets = OrtNets::load(&env_path("HM_STEMS_MODEL_DIR")).expect("load nets");
    let mut scratch = ChunkScratch::new();
    let mut compared = 0;
    for c in 0..8 {
        let v_ref = dir.join(format!("chunk{c}.vocals.f32"));
        if !v_ref.exists() {
            continue;
        }
        let d_ref = dir.join(format!("chunk{c}.drums.f32"));
        let out = separate_chunk(&mut nets, &mut scratch, &mix, c).expect("separate");
        let (rv, rd) = (read_f32(&v_ref), read_f32(&d_ref));
        assert_eq!(out.vocals.len(), rv.len(), "chunk {c} length");
        let (sv, sd) = (sdr_db(&rv, &out.vocals), sdr_db(&rd, &out.drums));
        let (ev, ed) = (max_abs(&rv, &out.vocals), max_abs(&rd, &out.drums));
        let energy: f64 = rv.iter().chain(&rd).map(|&v| f64::from(v).powi(2)).sum();
        println!(
            "chunk {c}: frames {} vocals SDR {sv:.1} dB (max err {ev:.2e})  drums SDR {sd:.1} dB (max err {ed:.2e})  ref vocals+drums energy {energy:.1}",
            rv.len() / 2
        );
        assert!(energy > 1.0, "reference is near silent: the test would prove nothing");
        assert!(sv > 40.0 && sd > 40.0, "chunk {c} below the 40 dB parity bar");
        compared += 1;
    }
    assert!(compared >= 2, "expected at least two reference chunks in {}", dir.display());
}

#[test]
#[ignore = "needs the nets and ORT"]
fn real_nets_partition_the_mix_and_report_speed() {
    let mix = read_f32(&env_path("HM_STEMS_PARITY_INPUT"));
    let frames = mix.len() / 2;
    let store = StemStore::new(mix.clone());
    let sep = hm_stems::Separator::new(env_path("HM_STEMS_MODEL_DIR"), std::env::temp_dir());
    let t0 = Instant::now();
    let mut first = None;
    sep.fill(&store, &AtomicBool::new(false), |ready, _| {
        first.get_or_insert(t0.elapsed());
        eprintln!("  chunk {ready}/{} at {:.2}s", store.chunk_total(), t0.elapsed().as_secs_f64());
    })
    .expect("fill");
    let took = t0.elapsed().as_secs_f64();
    let secs = frames as f64 / 44_100.0;
    println!(
        "separated {secs:.1} s of audio in {took:.2} s (incl. model load) = {:.1}x real time; first chunk after {:.2} s",
        secs / took,
        first.unwrap().as_secs_f64()
    );
    let (v, d) = store.assemble().expect("complete");
    assert_eq!(v.len(), mix.len());
    // Physical sanity: vocals + drums never exceed the mix's energy (the other
    // two masks take their share), and both are non-trivial.
    let e = |x: &[f32]| x.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>();
    let (em, ev, ed) = (e(&mix), e(&v), e(&d));
    println!("energy: mix {em:.1} vocals {ev:.1} drums {ed:.1}");
    assert!(ev > 0.0 && ed > 0.0 && ev + ed <= em * 1.05, "stems must partition, not add, energy");
    // chunk seams: no jump at the boundary between chunk 0 and 1.
    let b = owned_start(1) * 2;
    let jump = (v[b] - v[b - 2]).abs();
    let typical = (b - 2000..b).step_by(2).map(|i| (v[i + 2] - v[i]).abs()).fold(0.0, f32::max);
    println!("seam jump {jump:.5} vs local max step {typical:.5}");
    assert!(jump <= typical * 2.0 + 1e-3, "audible seam at the chunk boundary");
    let _ = owned_end(0);
}
