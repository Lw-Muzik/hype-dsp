# 10-band Multiband Compander — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Port the mobile Hype 10-band Linkwitz-Riley multiband compressor/expander into the desktop `hm-dsp` chain as a `Compander` stage with global controls + UI.

**Architecture:** New `Compander` `AudioProcessor`: 9 LR4 crossovers split into 10 bands (sequential), each band runs a ported single-band compressor/expander, bands sum back. Allocation-free `process` (scratch pre-sized in `prepare`), placed after Convolver / before Gain→Limiter.

**Tech Stack:** Rust (`hm-core` types, `hm-dsp` DSP), React+Zustand+TS UI, Tauri commands. No new crates.

## Global Constraints
- Real-time safety: `AudioProcessor::process` never allocates/locks/does IO. All band scratch pre-sized in `prepare`. Params read from the snapshot in `set_params`.
- Chain order: `Headphone → GraphicEq → Bass → Spatializer → Surround3D → Room → Convolver → Compander → Gain → Limiter`. Compander BEFORE Gain/Limiter (limiter stays the safety net).
- Disabled → bit-exact identity early return. Output clamp `(-4.0, 4.0)`. Denormal-flush band buffers/envelope (`room.rs` `flush` pattern).
- 10 bands, centers `[31,62,125,250,500,1000,2000,4000,8000,16000]` Hz; 9 crossovers at `sqrt(center[i]*center[i+1])`; LR4 = two cascaded Butterworth (Q=0.7071071) per LP and per HP, per channel.
- Global params applied to ALL bands (no per-band UI). `CompanderState` serde camelCase, mirrored in `src/lib/types.ts`.
- Defaults: enabled=false, threshold=-18, ratio=2.5, knee=8, attack=15ms, release=45ms, makeup=0, gate=-70, expander=2.0.
- Verification gates per touched crate: `cargo test -p <crate>`, `cargo clippy --all-targets -- -D warnings`, `pnpm tsc --noEmit` for TS.

---

## Task 1: `CompanderState` (hm-core) + EngineState + TS mirror

**Files:** Modify `crates/hm-core/src/types.rs`, `src/lib/types.ts`; Test in `types.rs`.

**Interfaces — Produces:** `hm_core::CompanderState { enabled: bool, threshold_db: f32, ratio: f32, knee_db: f32, attack_ms: f32, release_ms: f32, makeup_db: f32, gate_db: f32, expander_ratio: f32 }`, default disabled; field `EngineState.compander: CompanderState`.

- [ ] **Step 1: Failing test** — append to `types.rs` test module:
```rust
    #[test]
    fn compander_default_is_disabled_with_mastering_defaults() {
        let c = CompanderState::default();
        assert!(!c.enabled);
        assert_eq!(c.threshold_db, -18.0);
        assert_eq!(c.ratio, 2.5);
        assert_eq!(c.knee_db, 8.0);
        assert_eq!(c.attack_ms, 15.0);
        assert_eq!(c.release_ms, 45.0);
        assert_eq!(c.makeup_db, 0.0);
        assert_eq!(c.gate_db, -70.0);
        assert_eq!(c.expander_ratio, 2.0);
        assert!(!EngineState::default().compander.enabled);
    }
```
- [ ] **Step 2: Run → fail** `cargo test -p hm-core compander_default_is_disabled` (no `CompanderState`).
- [ ] **Step 3: Implement** — after `RoomState`'s Default impl in `types.rs`:
```rust
/// Multiband compander (10-band Linkwitz-Riley compressor/expander). Global
/// params are applied uniformly to every band. Ported from the mobile Hype MBC.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanderState {
    pub enabled: bool,
    /// Compression starts above this input level, in dB.
    pub threshold_db: f32,
    /// Compression ratio (1.0 = no compression).
    pub ratio: f32,
    /// Soft-knee width in dB (0 = hard knee).
    pub knee_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    /// Post (makeup) gain in dB.
    pub makeup_db: f32,
    /// Noise-gate threshold in dB; below it the expander engages.
    pub gate_db: f32,
    /// Expander ratio applied below the gate threshold (>= 1).
    pub expander_ratio: f32,
}

impl Default for CompanderState {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_db: -18.0,
            ratio: 2.5,
            knee_db: 8.0,
            attack_ms: 15.0,
            release_ms: 45.0,
            makeup_db: 0.0,
            gate_db: -70.0,
            expander_ratio: 2.0,
        }
    }
}
```
Add `pub compander: CompanderState,` to `EngineState` (after `pub convolver: ConvolverState,`) and `compander: CompanderState::default(),` to its `Default`.
- [ ] **Step 4: Run → pass** `cargo test -p hm-core compander_default_is_disabled`.
- [ ] **Step 5: TS mirror** — in `src/lib/types.ts` after `ConvolverState`:
```ts
/** Multiband compander (10-band LR compressor/expander); global params. */
export interface CompanderState {
  enabled: boolean;
  thresholdDb: number;
  ratio: number;
  kneeDb: number;
  attackMs: number;
  releaseMs: number;
  makeupDb: number;
  gateDb: number;
  expanderRatio: number;
}
```
Add `compander: CompanderState;` to the `EngineState` interface; if any default-state literal in TS needs it, add it with the defaults above.
- [ ] **Step 6:** `pnpm tsc --noEmit` clean.
- [ ] **Step 7: Commit** `feat(core): add CompanderState to EngineState + TS mirror`.

---

## Task 2: Butterworth `set_lowpass`/`set_highpass` on `Biquad`

**Files:** Modify `crates/hm-dsp/src/biquad.rs`; Test there.

**Interfaces — Consumes:** existing `Biquad` (`assign`, `process_sample`). **Produces:** `Biquad::set_lowpass(sample_rate, f0, q)`, `Biquad::set_highpass(sample_rate, f0, q)` (RBJ cookbook LPF/HPF).

- [ ] **Step 1: Failing tests** — add to `biquad.rs` tests:
```rust
    #[test]
    fn lowpass_passes_dc_attenuates_hf() {
        let mut lp = Biquad::identity();
        lp.set_lowpass(48_000.0, 1_000.0, 0.7071068);
        // Settle on DC ⇒ ~unity gain.
        let mut y = 0.0;
        for _ in 0..4000 { y = lp.process_sample(1.0); }
        assert!((y - 1.0).abs() < 0.05, "LP DC gain ~1, got {y}");
        // A 12 kHz tone (well above cutoff) is strongly attenuated.
        lp.reset();
        let mut peak = 0.0f32;
        for i in 0..4000 {
            let x = (2.0 * std::f32::consts::PI * 12_000.0 * i as f32 / 48_000.0).sin();
            peak = peak.max(lp.process_sample(x).abs());
        }
        assert!(peak < 0.3, "12kHz through 1kHz LP should be small, got {peak}");
    }

    #[test]
    fn highpass_blocks_dc_passes_hf() {
        let mut hp = Biquad::identity();
        hp.set_highpass(48_000.0, 1_000.0, 0.7071068);
        let mut y = 0.0;
        for _ in 0..4000 { y = hp.process_sample(1.0); }
        assert!(y.abs() < 0.05, "HP blocks DC, got {y}");
    }
```
- [ ] **Step 2: Run → fail** `cargo test -p hm-dsp lowpass_passes_dc` (no `set_lowpass`).
- [ ] **Step 3: Implement** — add to `impl Biquad` (RBJ LPF/HPF; reuse `assign`):
```rust
    /// Configure as an RBJ low-pass at `f0` with quality `q`.
    pub fn set_lowpass(&mut self, sample_rate: f32, f0: f32, q: f32) {
        let fs = sample_rate as f64;
        let f0 = (f0 as f64).clamp(1.0, fs * 0.495);
        let q = (q as f64).max(1e-4);
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let cos = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let b1 = 1.0 - cos;
        let b0 = b1 / 2.0;
        let b2 = b0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos;
        let a2 = 1.0 - alpha;
        self.assign(b0, b1, b2, a0, a1, a2);
    }

    /// Configure as an RBJ high-pass at `f0` with quality `q`.
    pub fn set_highpass(&mut self, sample_rate: f32, f0: f32, q: f32) {
        let fs = sample_rate as f64;
        let f0 = (f0 as f64).clamp(1.0, fs * 0.495);
        let q = (q as f64).max(1e-4);
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let cos = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let b1 = -(1.0 + cos);
        let b0 = (1.0 + cos) / 2.0;
        let b2 = b0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos;
        let a2 = 1.0 - alpha;
        self.assign(b0, b1, b2, a0, a1, a2);
    }
```
- [ ] **Step 4: Run → pass** `cargo test -p hm-dsp` (new + existing).
- [ ] **Step 5: Clippy + commit** `cargo clippy -p hm-dsp --all-targets -- -D warnings`; `feat(dsp): Butterworth low/high-pass on Biquad for LR crossovers`.

---

## Task 3: `compander.rs` — crossover + band compressor + stage (the core)

**Files:** Create `crates/hm-dsp/src/compander.rs`; Modify `crates/hm-dsp/src/lib.rs` (`pub mod compander;` + `pub use compander::Compander;`); Test in `compander.rs`.

**Interfaces — Consumes:** `Biquad` (incl. new LP/HP), `AudioProcessor`, `ProcessorParams`, `hm_core::CompanderState`. **Produces:** `pub struct Compander` impl `AudioProcessor`, `Compander::new(sample_rate, channels)`.

Implement the full module. Key pieces (port of `multiband_compressor.h` + `compressor.h`, allocation-free):

```rust
//! 10-band multiband compander — Linkwitz-Riley 4th-order crossovers split the
//! signal into 10 bands, each compressed/expanded by an independent dB-domain
//! compressor, then summed. Ported from the mobile Hype MBC (compressor.h +
//! multiband_compressor.h). Global params apply to every band.
//!
//! Real-time safe: all band scratch is pre-sized in `prepare`; `process` never
//! allocates/locks. LR4 crossovers are power-complementary so a flat (ratio 1,
//! no gate) compander reconstructs the input.

use crate::biquad::Biquad;
use crate::{AudioProcessor, ProcessorParams};

pub const BAND_COUNT: usize = 10;
const CROSSOVER_COUNT: usize = BAND_COUNT - 1; // 9
const CENTERS_HZ: [f32; BAND_COUNT] =
    [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
const BUTTERWORTH_Q: f32 = 0.707_107;
const LOG10_20: f32 = 8.685_889_6; // 20/ln(10)
const INV_LOG10_20: f32 = 0.115_129_255; // ln(10)/20
const GAIN_SMOOTH: f32 = 0.005;

#[inline]
fn flush(x: f32) -> f32 { if x.abs() < 1e-18 { 0.0 } else { x } }
#[inline]
fn db_to_lin(db: f32) -> f32 { (db * INV_LOG10_20).exp() }
#[inline]
fn lin_to_db(lin: f32) -> f32 { if lin < 1e-10 { -200.0 } else { lin.ln() * LOG10_20 } }

/// One LR4 crossover for one channel: two cascaded Butterworth LP + two HP.
#[derive(Clone, Copy)]
struct LrChannel { lp: [Biquad; 2], hp: [Biquad; 2] }
impl LrChannel {
    fn new() -> Self { Self { lp: [Biquad::identity(); 2], hp: [Biquad::identity(); 2] } }
    fn configure(&mut self, sr: f32, freq: f32) {
        for b in &mut self.lp { b.set_lowpass(sr, freq, BUTTERWORTH_Q); }
        for b in &mut self.hp { b.set_highpass(sr, freq, BUTTERWORTH_Q); }
    }
    fn reset(&mut self) { for b in self.lp.iter_mut().chain(self.hp.iter_mut()) { b.reset(); } }
    /// Split one sample into (low, high).
    #[inline]
    fn split(&mut self, x: f32) -> (f32, f32) {
        let low = self.lp[1].process_sample(self.lp[0].process_sample(x));
        let high = self.hp[1].process_sample(self.hp[0].process_sample(x));
        (low, high)
    }
}

/// Per-band single-band compressor/expander (dB-domain), stereo-linked.
struct BandCompressor {
    sample_rate: f32,
    env_db: f32,
    gain_smoothed_db: f32,
    attack_coeff: f32,
    release_coeff: f32,
    // cached params
    threshold: f32, ratio: f32, knee: f32,
    gate: f32, expander_ratio: f32,
    makeup_lin: f32,
}
impl BandCompressor {
    fn new(sample_rate: f32) -> Self {
        let mut s = Self {
            sample_rate, env_db: -96.0, gain_smoothed_db: 0.0,
            attack_coeff: 0.1, release_coeff: 0.001,
            threshold: -18.0, ratio: 2.5, knee: 8.0, gate: -70.0,
            expander_ratio: 2.0, makeup_lin: 1.0,
        };
        s.recalc(15.0, 45.0);
        s
    }
    fn recalc(&mut self, attack_ms: f32, release_ms: f32) {
        let a = (attack_ms * 0.001).max(0.001);
        let r = (release_ms * 0.001).max(0.001);
        self.attack_coeff = 1.0 - (-1.0 / (a * self.sample_rate)).exp();
        self.release_coeff = 1.0 - (-1.0 / (r * self.sample_rate)).exp();
    }
    fn set_params(&mut self, p: &ProcessorParams) {
        let c = &p.compander;
        self.threshold = c.threshold_db;
        self.ratio = c.ratio.max(1.0);
        self.knee = c.knee_db.max(0.0);
        self.gate = c.gate_db;
        self.expander_ratio = c.expander_ratio.max(1.0);
        self.makeup_lin = db_to_lin(c.makeup_db);
        self.recalc(c.attack_ms, c.release_ms);
    }
    fn reset(&mut self) { self.env_db = -96.0; self.gain_smoothed_db = 0.0; }
    /// dB gain change for an input level (≤0 compression / expansion).
    #[inline]
    fn compute_gain(&self, input_db: f32) -> f32 {
        let mut gain_db = 0.0;
        if input_db < self.gate {
            gain_db = -(self.gate - input_db) * (self.expander_ratio - 1.0);
        }
        if input_db > self.threshold {
            let over = input_db - self.threshold;
            let half_knee = self.knee * 0.5;
            if self.knee > 0.0 && over < half_knee {
                let x = over / half_knee;
                gain_db -= over * (1.0 - 1.0 / self.ratio) * x * 0.5;
            } else {
                let full_over = if self.knee > 0.0 { over - half_knee } else { over };
                if self.knee > 0.0 { gain_db -= half_knee * (1.0 - 1.0 / self.ratio) * 0.5; }
                gain_db -= full_over * (1.0 - 1.0 / self.ratio);
            }
        }
        gain_db
    }
    /// Process one stereo frame in place (peak-linked).
    #[inline]
    fn process_frame(&mut self, l: &mut f32, r: &mut f32) {
        let peak = l.abs().max(r.abs());
        let peak_db = lin_to_db(peak);
        if peak_db > self.env_db {
            self.env_db += self.attack_coeff * (peak_db - self.env_db);
        } else {
            self.env_db += self.release_coeff * (peak_db - self.env_db);
        }
        self.env_db = flush(self.env_db + 96.0) - 96.0; // keep env from denormal drift
        let gain_db = self.compute_gain(self.env_db);
        self.gain_smoothed_db += GAIN_SMOOTH * (gain_db - self.gain_smoothed_db);
        let g = db_to_lin(self.gain_smoothed_db) * self.makeup_lin;
        *l *= g;
        *r *= g;
    }
}

/// The 10-band compander stage.
pub struct Compander {
    sample_rate: f32,
    enabled: bool,
    crossovers_l: Vec<LrChannel>, // len CROSSOVER_COUNT
    crossovers_r: Vec<LrChannel>,
    bands: Vec<BandCompressor>,   // len BAND_COUNT
}

impl Compander {
    pub fn new(sample_rate: f32, _channels: usize) -> Self {
        let mut s = Self {
            sample_rate, enabled: false,
            crossovers_l: (0..CROSSOVER_COUNT).map(|_| LrChannel::new()).collect(),
            crossovers_r: (0..CROSSOVER_COUNT).map(|_| LrChannel::new()).collect(),
            bands: (0..BAND_COUNT).map(|_| BandCompressor::new(sample_rate)).collect(),
        };
        s.reconfigure();
        s
    }
    fn crossover_freq(i: usize) -> f32 { (CENTERS_HZ[i] * CENTERS_HZ[i + 1]).sqrt() }
    fn reconfigure(&mut self) {
        for i in 0..CROSSOVER_COUNT {
            let f = Self::crossover_freq(i);
            self.crossovers_l[i].configure(self.sample_rate, f);
            self.crossovers_r[i].configure(self.sample_rate, f);
        }
    }
}

impl AudioProcessor for Compander {
    fn prepare(&mut self, sample_rate: f32, _channels: usize) {
        self.sample_rate = sample_rate;
        for b in &mut self.bands { b.sample_rate = sample_rate; b.reset(); b.recalc(15.0, 45.0); }
        for c in self.crossovers_l.iter_mut().chain(self.crossovers_r.iter_mut()) { c.reset(); }
        self.reconfigure();
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize) {
        if !self.enabled || channels == 0 { return; }
        let frames = buffer.len() / channels;
        let stereo = channels >= 2;
        for f in 0..frames {
            let base = f * channels;
            let in_l = buffer[base];
            let in_r = if stereo { buffer[base + 1] } else { in_l };
            // Sequential split: rest_* carries the high path into the next crossover.
            let (mut rest_l, mut rest_r) = (in_l, in_r);
            let mut sum_l = 0.0;
            let mut sum_r = 0.0;
            for i in 0..CROSSOVER_COUNT {
                let (low_l, high_l) = self.crossovers_l[i].split(rest_l);
                let (low_r, high_r) = self.crossovers_r[i].split(rest_r);
                let (mut bl, mut br) = (low_l, low_r);
                self.bands[i].process_frame(&mut bl, &mut br);
                sum_l += bl; sum_r += br;
                rest_l = high_l; rest_r = high_r;
            }
            // Last band = the remaining high path.
            let (mut bl, mut br) = (rest_l, rest_r);
            self.bands[BAND_COUNT - 1].process_frame(&mut bl, &mut br);
            sum_l += bl; sum_r += br;

            let out_l = flush(sum_l).clamp(-4.0, 4.0);
            let out_r = flush(sum_r).clamp(-4.0, 4.0);
            buffer[base] = out_l;
            if stereo { buffer[base + 1] = out_r; }
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        self.enabled = params.compander.enabled;
        for b in &mut self.bands { b.set_params(params); }
    }
}
```
Tests (add a `#[cfg(test)] mod tests`):
- `disabled_is_identity` — default state (disabled) → buffer unchanged bit-exact.
- `flat_compander_reconstructs_input` — enabled with `ratio=1.0`, `gate_db=-200.0` (no expansion), `makeup_db=0`, `knee=0`; feed a multi-tone stereo signal, prime to settle, assert the summed output ≈ input within a tolerance (e.g. RMS error < 0.05) — verifies LR4 reconstruction + that a unity compressor is ~transparent.
- `loud_input_is_compressed` — high ratio + low threshold, sustained loud tone: output peak < input peak after settling.
- `stays_bounded` — hostile sustained input stays within ±4.0.
- `quiet_below_gate_is_expanded_down` — very quiet input with gate above it → output quieter than input.

Use helper builders for `EngineState { compander: CompanderState{..}, ..default() }`. Mirror `room.rs` test style.

- [ ] Steps: write tests → run (fail: no `Compander`) → implement module + register in lib.rs → run (pass) → `cargo clippy -p hm-dsp --all-targets -- -D warnings` → commit `feat(dsp): 10-band Linkwitz-Riley multiband compander`.

(If `flat_compander_reconstructs_input` shows a level offset, the cause is the gain-smoothing settling or the gate — ensure `ratio=1.0` makes `compute_gain` return 0 above threshold and `gate_db` very low disables expansion; prime several blocks before comparing. Do NOT loosen tolerance to hide a real reconstruction error — if LR4 sum is wrong, recheck the cascade.)

---

## Task 4: Insert `Compander` into `ProcessChain`

**Files:** Modify `crates/hm-dsp/src/lib.rs`; Test there.

**Interfaces — Consumes:** `Compander`. Inserted in `standard_with_ir` after the `Convolver` push, before `Gain`.

- [ ] **Step 1: Failing test** — add to lib.rs tests:
```rust
    #[test]
    fn standard_chain_includes_compander() {
        let chain = ProcessChain::standard_with_ir(48_000.0, 2, crate::empty_ir_slot());
        assert!(chain.len() >= 9, "compander should be in the standard chain");
    }
```
- [ ] **Step 2: Run → fail** (len is 9 before adding; adjust assert to current+1). First check current `standard_with_ir` length (it's 9 stages today: Headphone,GraphicEq,BassBoost,Spatializer,Surround3D,RoomEffects,Convolver,Gain,Limiter) → after adding Compander it's 10, so assert `>= 10`. Use `>= 10`.
- [ ] **Step 3: Implement** — in `standard_with_ir`, add after the `Convolver::with_slot(...)` push:
```rust
        chain.push(Box::new(Compander::new(sample_rate, channels)));
```
Add `Compander` to the `pub use compander::...` import. Update the `standard`/`standard_with_ir` doc-comment chain order to include `→ Compander`.
- [ ] **Step 4: Run → pass** `cargo test -p hm-dsp`.
- [ ] **Step 5: Clippy + commit** `feat(dsp): insert Compander into the standard chain after Convolver`.

---

## Task 5: Engine `set_compander` + Tauri command + registration

**Files:** Modify `crates/hm-audio/src/engine.rs`, `src-tauri/src/commands/engine.rs`, `src-tauri/src/lib.rs`.

**Interfaces — Produces:** `AudioEngine::set_compander(CompanderState)`; command `engine_set_compander(compander: CompanderState)`.

- [ ] **Step 1:** In `engine.rs` near `set_room`, add (clamp the ratios/knee like `set_room` clamps):
```rust
    /// Configure the multiband compander stage.
    pub fn set_compander(&self, mut compander: hm_core::CompanderState) {
        compander.ratio = compander.ratio.max(1.0);
        compander.expander_ratio = compander.expander_ratio.max(1.0);
        compander.knee_db = compander.knee_db.max(0.0);
        compander.attack_ms = compander.attack_ms.max(0.1);
        compander.release_ms = compander.release_ms.max(0.1);
        self.update(|s| s.compander = compander);
    }
```
Add `CompanderState` to the `hm_core::{...}` import in engine.rs.
- [ ] **Step 2:** In `src-tauri/src/commands/engine.rs` after `engine_set_room`:
```rust
/// Configure the multiband compander stage.
#[tauri::command]
pub fn engine_set_compander(engine: State<'_, AudioEngine>, compander: hm_core::CompanderState) {
    engine.set_compander(compander);
}
```
- [ ] **Step 3:** Register `commands::engine::engine_set_compander,` in `src-tauri/src/lib.rs` handler list (after `engine_set_room` or the convolver commands).
- [ ] **Step 4:** `cargo build -p hypemuzik` compiles; `cargo test -p hm-audio` green; `cargo clippy -p hm-audio --all-targets -- -D warnings` clean.
- [ ] **Step 5: Commit** `feat(tauri): engine_set_compander command`.

---

## Task 6: TS IPC + store + `CompanderCard.tsx`

**Files:** Modify `src/lib/ipc.ts`, `src/stores/engine.ts`, `src/features/enhancer/EnhancerView.tsx`; Create `src/features/enhancer/CompanderCard.tsx`.

**Interfaces — Produces (store):** `setCompander(next: CompanderState)`.

- [ ] **Step 1: IPC** — in `ipc.ts` near `engineSetRoom`:
```ts
export function engineSetCompander(compander: CompanderState): Promise<void> {
  return invoke<void>("engine_set_compander", { compander });
}
```
Add `CompanderState` to the `@/lib/types` import.
- [ ] **Step 2: Store** — in `engine.ts`, add to type block + impl (mirror `setRoom`):
```ts
  setCompander: (next: CompanderState) => void;
```
```ts
    setCompander: (next) => {
      set((s) => ({ state: { ...s.state, compander: next } }));
      void engineSetCompander(next).catch(() => {});
    },
```
Add `engineSetCompander` to the `@/lib/ipc` import and `CompanderState` to the `@/lib/types` import.
- [ ] **Step 3: Card** — create `CompanderCard.tsx` mirroring `RoomCard.tsx` EXACTLY for component usage (verify props against RoomCard: `Card` with `title`/`icon`, `Switch` `checked`+`onChange`, `Slider` `value/min/max/step/onChange/formatValue/className`). Every `<Slider>` MUST have a width class (`flex-1`). Controls: enable Switch + sliders for threshold (−60..0 dB), ratio (1..20), knee (0..24 dB), attack (1..200 ms), release (10..1000 ms), makeup (0..24 dB), gate (−90..−20 dB), expander ratio (1..10). Optional macro preset buttons ("Night mode": threshold −30, ratio 6, makeup +6; "Punch": ratio 1.5, expander 3) that call `setCompander` with a merged state. Use the dB/ms/ratio formatters inline.
```tsx
import { Gauge } from "lucide-react";
import { Card } from "@/components/Card";
import { Switch } from "@/components/Switch";
import { Slider } from "@/components/Slider";
import { useEngineStore } from "@/stores/engine";
import { cn } from "@/lib/cn";

type Key = "thresholdDb"|"ratio"|"kneeDb"|"attackMs"|"releaseMs"|"makeupDb"|"gateDb"|"expanderRatio";
interface Def { key: Key; label: string; min: number; max: number; step: number; fmt: (v:number)=>string }
const db = (v:number)=>`${v.toFixed(1)} dB`;
const ms = (v:number)=>`${Math.round(v)} ms`;
const x = (v:number)=>`${v.toFixed(1)}:1`;
const SLIDERS: readonly Def[] = [
  { key:"thresholdDb", label:"Threshold", min:-60, max:0, step:0.5, fmt:db },
  { key:"ratio", label:"Ratio", min:1, max:20, step:0.1, fmt:x },
  { key:"kneeDb", label:"Knee", min:0, max:24, step:0.5, fmt:db },
  { key:"attackMs", label:"Attack", min:1, max:200, step:1, fmt:ms },
  { key:"releaseMs", label:"Release", min:10, max:1000, step:5, fmt:ms },
  { key:"makeupDb", label:"Makeup", min:0, max:24, step:0.5, fmt:db },
  { key:"gateDb", label:"Gate", min:-90, max:-20, step:1, fmt:db },
  { key:"expanderRatio", label:"Expander", min:1, max:10, step:0.1, fmt:x },
];

export function CompanderCard() {
  const c = useEngineStore((s) => s.state.compander);
  const setCompander = useEngineStore((s) => s.setCompander);
  return (
    <Card title="Multiband compander" icon={Gauge}
      actions={<Switch checked={c.enabled} onChange={(enabled) => setCompander({ ...c, enabled })} />}>
      <div className={cn("flex flex-col gap-3", !c.enabled && "opacity-60")}>
        {SLIDERS.map((d) => (
          <div key={d.key} className="flex items-center gap-3">
            <span className="w-20 shrink-0 text-sm text-text-muted">{d.label}</span>
            <Slider className="flex-1" min={d.min} max={d.max} step={d.step}
              value={c[d.key]} formatValue={d.fmt}
              onChange={(v) => setCompander({ ...c, [d.key]: v })} />
            <span className="w-16 text-right text-xs tabular-nums text-text-muted">{d.fmt(c[d.key])}</span>
          </div>
        ))}
      </div>
    </Card>
  );
}
```
(Match `Card`'s real prop names against `RoomCard.tsx` — if it uses a different slot than `actions` for the switch, follow RoomCard.)
- [ ] **Step 4: Render** — in `EnhancerView.tsx`, import `CompanderCard` and render `<CompanderCard />` after `<ConvolverCard />` (or after `<RoomCard />` if Convolver card isn't there).
- [ ] **Step 5:** `pnpm tsc --noEmit` clean.
- [ ] **Step 6: Commit** `feat(ui): CompanderCard — multiband dynamics controls + store wiring`.

---

## Final verification
- `cargo test --workspace` green; `cargo clippy --all-targets -- -D warnings` clean; `pnpm tsc --noEmit` clean.
- Manual smoke (if dev build): enable compander, raise ratio + lower threshold on loud music → audibly evens out; "Night mode" preset; confirm no glitches and UI responsive.

## Self-review notes
- Spec coverage: state (T1), Butterworth biquads (T2), crossover+compressor+stage (T3), chain insert (T4), engine+command (T5), UI (T6). All map.
- RT-safety: no alloc in `process` (fixed-size per-frame locals, persistent crossover/band state), disabled early-return, denormal flush + clamp.
- Type consistency: `CompanderState`/`compander` field, `Compander`, `set_compander`, `engine_set_compander`, `setCompander` used identically across tasks.
- Confirms the implementer must make: `Card`/`Switch`/`Slider` prop names vs RoomCard (T6); current `standard_with_ir` stage count for the chain test assert (T4).
