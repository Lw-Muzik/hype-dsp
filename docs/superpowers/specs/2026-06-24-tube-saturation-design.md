# Tube Saturation (4× oversampled) — Design

**Date:** 2026-06-24
**Status:** Approved (4× oversampling chosen by user)
**Scope:** Add a wideband tube-style analog-saturation stage to the desktop `hm-dsp` chain — JamesDSP "tube modeling" gap (#4). Asymmetric soft saturation emphasizing 2nd-harmonic warmth, 4× oversampled for clean (alias-free) harmonics, with dry/wet mix and auto output-level compensation.

## 1. Goal
A musical "analog warmth" color: gentle even-harmonic (2nd) saturation across the whole band (distinct from the lows-only harmonic enhancement already in `bass_boost`). Mastering-grade cleanliness — 4× oversampling so the generated harmonics never alias into harsh inharmonic tones even at high drive. Enabling it must not jump the level ("nothing distorts").

## 2. Signal path (per channel)
```
dry ─┬─────────────────────────────────────────────┐
     └─►[4× upsample FIR]►[tube waveshaper]►[DC block]►[4× downsample FIR]─► wet ─►[×makeup]─┐
                                                                                              ├─►(dry·(1-mix) + wet·mix)►clamp
dry ──────────────────────────────────────────────────────────────────────────────────────┘
```
- **4× oversampler:** windowed-sinc (Blackman/Hamming) lowpass FIR, cutoff at the ORIGINAL Nyquist (= 0.125 of the oversampled Nyquist). Upsample = zero-stuff ×4 + FIR (×4 gain to preserve level); downsample = FIR + decimate ×4. **Polyphase** implementation (compute only non-zero taps) for efficiency — the user accepted the CPU cost but we still avoid the naive 4× redundancy. ~64-tap FIR (4 phases × 16) for a clean stopband. Latency = FIR group delay (reported, but small; acceptable for a player).
- **Tube waveshaper:** asymmetric soft clip rich in 2nd harmonic. `y = tanh(drive·(x + bias)) − tanh(drive·bias)` — the `bias` makes it asymmetric (even harmonics), the constant subtraction removes the static DC the bias introduces. `drive` scales the amount. A small fixed `bias` (e.g. 0.1–0.3) gives tube character without excessive asymmetry.
- **DC blocker:** one-pole high-pass (~10–20 Hz) after the shaper to remove residual DC from the asymmetry (asymmetric shaping always leaves some DC).
- **Auto makeup:** `tanh` reduces peaks as drive rises, so the wet path is scaled by a **drive-dependent static compensation** (a smooth function so there's no level jump or pumping — NOT a level-dependent AGC, which would pump). Goal: enabling saturation at a given drive keeps perceived loudness ≈ constant.
- **Mix:** `out = dry·(1−mix) + wet·mix·makeup`, then `clamp(-4,4)`. Dry path is the undelayed input — the wet path's FIR latency means a slight phase offset between dry and wet; with a short FIR this is minor, but the design delays the DRY path by the FIR group delay so dry/wet stay time-aligned (a small fixed delay line on dry). (If group delay is integer samples, this is exact.)

## 3. Placement
Insert in `ProcessChain::standard_with_ir` **after `Compander`, before `Gain → Limiter`**: warms the fully-processed mix; the master −0.3 dBFS limiter remains the final safety net → "nothing distorts" holds. New chain order: `Headphone → GraphicEq → Bass → Spatializer → Surround3D → Room → Convolver → Compander → Saturation → Gain → Limiter`.

## 4. Real-time safety & "nothing distorts"
- `process()` allocation-free: oversampled scratch buffers, FIR delay lines, and the dry-delay line are all pre-sized in `prepare` (resized only off the steady path if a bigger block arrives). Params read from the snapshot in `set_params` (change-guarded).
- Disabled → bit-exact identity early return (zero cost). Output `clamp(-4,4)`; denormal-flush the FIR/DC-block state. Master limiter downstream.
- Auto makeup is a smooth static function of drive (no zipper); mix change is per-block param, smoothed if needed.

## 5. State / params
`SaturationState` (hm-core, serde camelCase, `Default`=disabled):
- `enabled: bool`
- `drive: f32` (0..1 → mapped internally to a tanh drive range, e.g. 1..~10)
- `mix: f32` (0..1 dry/wet)
- (bias + makeup are internal, derived from drive — not user params; YAGNI)
Defaults: enabled=false, drive=0.3, mix=1.0. Added as `EngineState.saturation`. Engine `set_saturation`, Tauri `engine_set_saturation`, store `setSaturation`, `SaturationCard.tsx` (enable + Drive + Mix sliders + optional "warm/hot" macro presets).

## 6. Files
- `crates/hm-dsp/src/oversample.rs` — `Oversampler4x` (per-channel polyphase 4× up/down FIR) + the FIR design. Reusable (future stages could oversample too).
- `crates/hm-dsp/src/saturation.rs` — waveshaper + DC blocker + `Saturation` `AudioProcessor` (owns 2× `Oversampler4x` for L/R, dry-delay, makeup).
- `hm-core/types.rs` `SaturationState` (+TS mirror), chain insert in `hm-dsp/lib.rs`, engine/command/store/card.

## 7. Testing
- **Oversampler** (the riskiest — test hard): a sub-Nyquist sine (e.g. 1 kHz @48k) through up→down with NO waveshaping returns ~same amplitude (within ~2–3%), delayed by the FIR group delay (passband flatness + round-trip correctness). A near-Nyquist tone is attenuated by the FIR (stopband). Round-trip of broadband noise preserves in-band energy.
- **Waveshaper:** monotonic, bounded; asymmetric bias produces measurable 2nd-harmonic (even) energy (FFT a sine, check 2f bin > 3f bin region for the asymmetric shaper); drive=0 (or the disabled stage) = passthrough.
- **Anti-aliasing:** a high tone (e.g. 15 kHz @48k) at high drive, 4× oversampled, has LESS energy in the low-frequency alias region than a single-rate reference saturator (demonstrates the oversampling works). (If a rigorous version is hard, at least assert the saturated output is bounded and the in-band fundamental survives.)
- **Stage:** `disabled_is_identity` bit-exact; auto-makeup keeps RMS roughly stable across drive on a steady tone (enabling doesn't jump level); `stays_bounded` under hostile input; dry/wet at mix=0 = dry (delayed), mix=1 = wet.
- Gates: `cargo test -p hm-dsp`/`-p hm-core`, `cargo clippy --all-targets -- -D warnings`, `pnpm tsc --noEmit`.

## 8. Build order (tasks)
1. `SaturationState` (hm-core) + EngineState + TS mirror.
2. `oversample.rs` — `Oversampler4x` (polyphase FIR up/down) + tests (the hard part).
3. `saturation.rs` — waveshaper + DC blocker + `Saturation` stage (uses oversampler, dry-delay, makeup) + tests.
4. Chain insert (after Compander) + chain test.
5. Engine `set_saturation` + Tauri command + register.
6. TS IPC + store + `SaturationCard` + render in EnhancerView.

## 9. Non-goals (YAGNI)
Tube/triode/pentode model selection; tone/bias UI controls; per-band saturation; variable oversampling factor (fixed 4×); tape-style wow/flutter. Future if asked.

## 10. Risks
- **FIR/oversampler correctness** — the hardest part; mitigated by the passband/stopband/round-trip tests. A wrong FIR gain → level error; wrong cutoff → aliasing or HF loss.
- **Latency from the FIR** — small; dry path delayed to match so no comb filtering between dry/wet. Adds to total chain latency (convolver + compander lookahead + this); fine for a player, note it.
- **Aliasing despite oversampling at extreme drive** — 4× handles typical use; extreme drive could still alias slightly, bounded by the limiter. Acceptable.
- **CPU** — 4× + 64-tap polyphase FIR × 2ch; the user accepted the cost. Disabled = zero cost.
