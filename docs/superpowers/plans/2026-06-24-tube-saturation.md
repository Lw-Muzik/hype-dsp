# Tube Saturation (4× oversampled) — Implementation Plan

> REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Checkbox steps.

**Goal:** A wideband tube-style saturation stage (asymmetric 2nd-harmonic warmth), 4× oversampled for alias-free harmonics, dry/wet mix + auto makeup, in the desktop `hm-dsp` chain.

**Architecture:** New `Oversampler4x` (polyphase windowed-sinc FIR up/down) + a `Saturation` `AudioProcessor` (tube waveshaper + DC blocker, dry-delay-aligned wet path, drive-based makeup), inserted after Compander / before Gain→Limiter. Allocation-free `process`.

**Tech Stack:** Rust hm-dsp/hm-core; React+Zustand+TS; Tauri. No new crates.

## Global Constraints
- RT-safety: `process()` never allocates/locks/IO; all FIR/oversample/dry-delay buffers pre-sized in `new`/`prepare`. Params via snapshot in `set_params` (change-guarded).
- Disabled → bit-exact identity. Output clamp(-4,4). Denormal-flush FIR/DC-block state.
- Chain order becomes: Headphone → GraphicEq → Bass → Spatializer → Surround3D → Room → Convolver → Compander → **Saturation** → Gain → Limiter (Saturation BEFORE Gain/Limiter; limiter = safety net).
- `SaturationState` serde camelCase, mirrored in `src/lib/types.ts`. Defaults: enabled=false, drive=0.3, mix=1.0.
- Gates per crate: `cargo test -p <crate>`, `cargo clippy --all-targets -- -D warnings`, `pnpm tsc --noEmit`.

---

## Task 1: `SaturationState` (hm-core) + EngineState + TS mirror
**Files:** Modify `crates/hm-core/src/types.rs`, `src/lib/types.ts`; test in types.rs.
**Produces:** `hm_core::SaturationState { enabled: bool, drive: f32, mix: f32 }` default disabled; `EngineState.saturation`.

- [ ] Failing test:
```rust
    #[test]
    fn saturation_default_is_disabled() {
        let s = SaturationState::default();
        assert!(!s.enabled);
        assert_eq!(s.drive, 0.3);
        assert_eq!(s.mix, 1.0);
        assert!(!EngineState::default().saturation.enabled);
    }
```
- [ ] Run → fail. Implement (after another state's Default in types.rs):
```rust
/// Tube-style analog saturation (4× oversampled, 2nd-harmonic warmth).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaturationState {
    pub enabled: bool,
    /// 0..1 → internal tanh drive amount.
    pub drive: f32,
    /// Dry/wet mix, 0..1.
    pub mix: f32,
}
impl Default for SaturationState {
    fn default() -> Self { Self { enabled: false, drive: 0.3, mix: 1.0 } }
}
```
Add `pub saturation: SaturationState,` to `EngineState` (after `compander`) + `saturation: SaturationState::default(),` to its Default.
- [ ] Run → pass. TS mirror in types.ts:
```ts
/** Tube-style analog saturation (4× oversampled). */
export interface SaturationState { enabled: boolean; drive: number; mix: number; }
```
Add `saturation: SaturationState;` to `EngineState`; fix any default-state literal. `pnpm tsc --noEmit` clean. Commit `feat(core): add SaturationState to EngineState + TS mirror`.

---

## Task 2: `Oversampler4x` — polyphase windowed-sinc 4× up/down FIR (the hard part)
**Files:** Create `crates/hm-dsp/src/oversample.rs`; Modify `crates/hm-dsp/src/lib.rs` (`mod oversample;`); test in oversample.rs.
**Produces:** `pub struct Oversampler4x` (single channel) with `new(sample_rate)`, `reset()`, and a way to process a block: `upsample(&mut self, input: &[f32], out4x: &mut [f32])` (out4x.len() == input.len()*4) and `downsample(&mut self, in4x: &[f32], out: &mut [f32])` (out.len() == in4x.len()/4). `pub fn latency_samples(&self) -> usize` (group delay at the base rate). `pub const OVERSAMPLE: usize = 4;`.

### Design (specify, derive coefficients in code — do NOT hardcode magic arrays)
- FIR is a **windowed-sinc lowpass** computed in `new()`: `NUM_TAPS` taps (use a multiple of 4, e.g. 64), cutoff `fc = 0.25 / OVERSAMPLE` of the *oversampled* sample rate? — be precise: the FIR runs at the 4× rate and must pass the original band [0, base_nyquist] and stop above it. So normalized cutoff (cycles/sample at the 4× rate) `fc = 0.5 / OVERSAMPLE = 0.125`. Tap `h[n] = sinc(2·fc·(n − M)) · window(n)`, `M=(NUM_TAPS-1)/2`, `sinc(x)=sin(πx)/(πx)` (with the `x==0 → 1` limit), `window` = Blackman or Hamming. Normalize so the DC gain (sum of taps) = 1 for the downsampler; the **upsampler** scales by `OVERSAMPLE` (=4) to preserve level after zero-stuffing.
- **Polyphase:** decompose `h` into 4 phases `h_p[k] = h[4k + p]` (p=0..3). 
  - **Upsample** input x[n] → 4 outputs y[4n+p] = Σ_k h_p_up[k]·x[n−k] (each phase is an FIR on the base-rate input; this is the efficient polyphase interpolator — no zero multiplies). Use the `×4`-scaled taps for the up phases.
  - **Downsample** in4x → out[m] = Σ over the 4 phases of (phase-p FIR applied to the corresponding decimated stream). Standard polyphase decimator. Use the unity-DC taps.
  - Maintain per-phase delay-line state (Vec, sized in new). If polyphase is too fiddly, a DIRECT zero-stuff+FIR upsample and FIR+decimate downsample is ACCEPTABLE (correct, just more mults) — correctness first; note which you used.
- All buffers (delay lines, scratch) allocated in `new`; `reset()` zeros them. Denormal-flush optional (FIR has finite memory, less critical, but flush if you keep IIR-like state — here it's FIR so not needed).

### Tests (the correctness gate — make them real)
```
- roundtrip_passband_unity: a 1 kHz sine @48k base, upsample then (no shaping) downsample, returns ~same amplitude (peak within 3%) after discarding the first `latency_samples()*2`-ish warmup. (Tests FIR gain + passband flatness + round-trip.)
- near_nyquist_attenuated: a tone near base Nyquist (e.g. 22 kHz @48k) is attenuated by the round-trip (stopband). 
- upsample_then_downsample_preserves_low_freq_energy: broadband-ish low content RMS preserved within a few %.
- dc_gain_unity: a DC/constant input round-trips to ~the same constant (FIR DC gain correct).
- latency_reported: latency_samples() > 0 and matches the FIR group delay you implemented.
```
Do NOT loosen these to hide a wrong FIR gain/cutoff — a failing roundtrip means the coefficients/scaling are wrong.

- [ ] TDD: tests → fail → implement → pass → `cargo clippy -p hm-dsp --all-targets -- -D warnings` → commit `feat(dsp): 4x polyphase windowed-sinc oversampler`.

---

## Task 3: `saturation.rs` — waveshaper + DC blocker + `Saturation` stage
**Files:** Create `crates/hm-dsp/src/saturation.rs`; Modify `crates/hm-dsp/src/lib.rs` (`pub mod saturation;` + `pub use saturation::Saturation;`); test there.
**Consumes:** `Oversampler4x`, `Biquad`/one-pole, `AudioProcessor`, `hm_core::SaturationState`.
**Produces:** `pub struct Saturation` impl `AudioProcessor`, `Saturation::new(sample_rate, channels)`.

### Pieces
- **Tube waveshaper** (per oversampled sample): `fn shape(x: f32, drive: f32, bias: f32) -> f32 { (drive*(x+bias)).tanh() - (drive*bias).tanh() }`. `drive` mapped from the 0..1 param to e.g. `1.0 + state.drive*9.0` (1..10); `bias` fixed ~0.2. Asymmetry → 2nd harmonic; the subtraction removes static DC.
- **DC blocker** (per channel, at base rate after downsample, OR at 4× before downsample): one-pole high-pass `y[n] = x[n] - x[n-1] + R*y[n-1]`, `R≈0.999` (~5–20 Hz). Removes residual DC from the asymmetric shaper. Denormal-flush its state.
- **Auto makeup**: a smooth static function of drive that compensates the peak reduction from tanh, so enabling at a given drive keeps loudness ≈constant. E.g. `makeup = 1.0 / shape_peak_estimate(drive)` or a simple `makeup = 1.0 + k*state.drive`. Keep it click-free (recompute only in set_params, change-guarded). Document the choice.
- **Dry-delay**: delay the DRY path by the oversampler's `latency_samples()` so dry/wet are time-aligned for the mix (a small per-channel delay line, sized in prepare). Integer-sample delay.
- **`Saturation` stage**: owns 2× `Oversampler4x` (L/R), DC blockers, dry-delay lines, oversampled scratch (sized in prepare for max block). `process`: de-interleave (mono mirror) → per channel: dry→dry-delay; wet = downsample(shape(upsample(in))) → DC-block → ; mix `out = dry_delayed*(1-mix) + wet*mix*makeup`; clamp(-4,4); re-interleave. Disabled → bit-exact identity early return. `set_params` reads `params.saturation` (change-guarded: enabled, drive, mix, recompute makeup/drive-map only on change).

### Tests
```
- disabled_is_identity: bit-exact.
- mix_zero_is_dry_delayed: mix=0, enabled → output equals the input delayed by latency (within FP eps) — dry path correct.
- produces_even_harmonic: enabled, drive high, mix=1, feed a pure sine; FFT the output — the 2nd-harmonic (2f) bin has clearly more energy than with the stage disabled (asymmetric shaping adds even harmonics). (Use realfft like graphic_eq's test.)
- makeup_keeps_level_roughly_stable: a steady sine at moderate drive, enabled output RMS within ~±2 dB of input RMS (no big level jump on enable).
- stays_bounded: hostile input stays within ±4.
- antialias_smoke (optional but encouraged): a 15 kHz tone @48k at high drive — assert output is bounded and the 15 kHz fundamental survives (full anti-alias proof is hard to unit-test; bounded + fundamental-present is the floor).
```
- [ ] TDD → implement → pass → `cargo clippy -p hm-dsp --all-targets -- -D warnings` → commit `feat(dsp): tube saturation stage (4x oversampled waveshaper + DC block + makeup)`.

---

## Task 4: Insert `Saturation` into `ProcessChain`
**Files:** Modify `crates/hm-dsp/src/lib.rs`; test there.
- [ ] Failing test: `standard_with_ir(...).len() >= 11` (chain is 10 after compander; +Saturation = 11). Verify the current count first.
- [ ] In `standard_with_ir`, push `Saturation::new(sample_rate, channels)` AFTER the `Compander::...` push, BEFORE `Gain::new()`. Add `Saturation` to imports; update the chain-order doc comment (crate-level `//!` AND the method doc) to include `→ Saturation`.
- [ ] `cargo test -p hm-dsp`, `cargo build -p hm-audio` (system-eq callers unaffected), clippy clean. Commit `feat(dsp): insert Saturation into the standard chain after Compander`.

---

## Task 5: Engine `set_saturation` + Tauri command
**Files:** `crates/hm-audio/src/engine.rs`, `src-tauri/src/commands/engine.rs`, `src-tauri/src/lib.rs`.
- [ ] engine.rs near `set_compander`:
```rust
    pub fn set_saturation(&self, mut saturation: hm_core::SaturationState) {
        saturation.drive = saturation.drive.clamp(0.0, 1.0);
        saturation.mix = saturation.mix.clamp(0.0, 1.0);
        self.update(|s| s.saturation = saturation);
    }
```
Add `SaturationState` to the `hm_core::{...}` import.
- [ ] commands/engine.rs after `engine_set_compander`:
```rust
#[tauri::command]
pub fn engine_set_saturation(engine: State<'_, AudioEngine>, saturation: hm_core::SaturationState) {
    engine.set_saturation(saturation);
}
```
- [ ] Register `commands::engine::engine_set_saturation,` in lib.rs handler list.
- [ ] `cargo build -p hypemuzik`, `cargo test -p hm-audio`, clippy clean. Commit `feat(tauri): engine_set_saturation command`.

---

## Task 6: TS IPC + store + `SaturationCard`
**Files:** `src/lib/ipc.ts`, `src/stores/engine.ts`, create `src/features/enhancer/SaturationCard.tsx`, modify `EnhancerView.tsx`.
- [ ] ipc.ts: `export function engineSetSaturation(saturation: SaturationState): Promise<void> { return invoke<void>("engine_set_saturation", { saturation }); }` (+ `SaturationState` import).
- [ ] engine.ts store: add `setSaturation: (next: SaturationState) => void;` to the type + impl mirroring `setRoom`/`setCompander`:
```ts
    setSaturation: (next) => {
      set((s) => ({ state: { ...s.state, saturation: next } }));
      void engineSetSaturation(next).catch(() => {});
    },
```
(+ imports.)
- [ ] `SaturationCard.tsx`: mirror `CompanderCard`/`RoomCard` EXACTLY for component props (Card `title`/`icon`/`actions`, Switch `checked`+`onChange`, Slider `value/min/max/step/onChange/formatValue/className`, label). Controls: enable Switch + Drive slider (0..1, show %) + Mix slider (0..1, show %). Every `<Slider>` has `className="flex-1"`. Optional "Warm"/"Hot" presets (Warm: drive 0.2; Hot: drive 0.6). Pick a lucide icon (e.g. `Flame`).
- [ ] Render `<SaturationCard />` after `<CompanderCard />` in EnhancerView.
- [ ] `pnpm tsc --noEmit` clean. Commit `feat(ui): SaturationCard — drive/mix + store wiring`.

---

## Final
- `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings` (4 crates), `pnpm tsc --noEmit` — green. Whole-branch review.

## Self-review notes
- Spec coverage: state(T1), oversampler(T2), waveshaper+stage(T3), chain(T4), engine+cmd(T5), UI(T6).
- RT-safety: no alloc in process (oversample/FIR/dry-delay pre-sized); disabled identity; clamp; flush.
- Risk: the oversampler FIR is the crux — Task 2's roundtrip/passband/DC tests are the gate; do not weaken them.
- Type consistency: `SaturationState`/`saturation`, `Saturation`, `set_saturation`, `engine_set_saturation`, `setSaturation`.
