# 10-band Multiband Compander — Design

**Date:** 2026-06-23
**Status:** Approved (band count + scope chosen by user)
**Scope:** Port the mobile Hype app's 10-band Linkwitz-Riley multiband compressor/expander into the desktop `hm-dsp` chain as a new `Compander` stage, with global (all-band) controls, a Tauri command, and a UI card.

## 1. Goal
Add multiband dynamics processing — compress to even out loudness ("night mode") or expand below a gate, per frequency band — closing the biggest functional gap vs JamesDSP (today the desktop has only a brickwall limiter). Faithfully port the proven mobile algorithm (`android/app/src/main/cpp/multiband_compressor.h` + `compressor.h`).

## 2. Source of truth (the mobile port)
- **10 bands**, 9 Linkwitz-Riley 4th-order crossovers (24 dB/oct = two cascaded Butterworth Q=0.7071 biquads per LP/HP), **sequential split** topology (split at f0 → band0 + rest; split rest at f1 → band1 + rest; …; last rest = band9).
- Crossover frequencies = geometric means of adjacent band centers from `[31,62,125,250,500,1k,2k,4k,8k,16k]` Hz → 9 crossovers at `sqrt(f[i]*f[i+1])`.
- **Per-band compressor** (Chromium DynamicsCompressorKernel-derived): per-sample peak detect (max |L|,|R| for stereo coherence), dB-domain envelope follower (1-pole attack/release), gain computer (threshold/ratio/soft-knee + noise-gate/expander below gate), gain smoothing (`gainSmoothedDb += 0.005*(gainDb - gainSmoothedDb)`), pre/post gain. dB↔lin via the same constants.
- Bands summed after compression (LR4 is power-complementary, so a flat/disabled compressor sums back to ~unity).

## 3. Deviations from the mobile code (deliberate, required)
1. **No allocation in `process()`** — the mobile version `new float[]`s per block. The desktop `AudioProcessor::process` contract forbids heap/lock/IO on the audio thread, so **all band scratch buffers are pre-sized in `prepare()`** (resized only if a larger-than-seen block arrives, off the steady path). This is the same discipline as the convolver/room stages.
2. **Interleaved buffer** — the chain hands `&mut [f32]` interleaved by `channels`; the stage de-interleaves into per-band L/R scratch and re-interleaves on sum (mirrors `room.rs`). Mono (channels==1) handled by mirroring.
3. **Global params, not per-band UI** — the mobile app drives all 10 bands uniformly via its `setAll*` surface; we expose the same global knobs (no 90-control UI). The 10-band crossover still does real multiband splitting internally.
4. **Reuse `hm-dsp` `Biquad`** — add Butterworth `set_lowpass`/`set_highpass` (RBJ LPF/HPF, Q=0.7071) to the existing `biquad.rs`; the crossover cascades two per LP and two per HP, per channel.

## 4. Architecture
- New `crates/hm-dsp/src/compander.rs` implementing `AudioProcessor`:
  - `LrCrossover` — per channel: `lp: [Biquad; 2]`, `hp: [Biquad; 2]`; `split_sample(l,r) -> (low_l, low_r, high_l, high_r)` (or per-channel `split(x)->(low,high)`).
  - `BandCompressor` — the ported single-band compressor (envelope/gain-computer/smoothing); `process_frame(l, r) -> (l', r')`.
  - `Compander` — owns `[LrCrossover; 9]` + `[BandCompressor; 10]` + pre-sized scratch; `prepare` (re)builds crossovers for the sample rate; `set_params` pushes global compressor params to all bands (change-guarded); `process` does sequential split → per-band compress → sum, in place, allocation-free, denormal-flushed, output clamped.
- Inserted in `ProcessChain::standard_with_ir` **after `Convolver`, before `Gain → Limiter`** (dynamics before makeup/brickwall; master limiter stays the final safety net → "nothing distorts" holds).
- `CompanderState` in `hm-core/types.rs` added to `EngineState` (+ TS mirror), serde camelCase, `Default` = disabled with sensible mastering defaults (threshold −18 dB, ratio 2.5, knee 8 dB, attack 15 ms, release 45 ms, makeup 0 dB, gate −70 dB, expander 2.0). Engine method `set_compander`, Tauri command `engine_set_compander`, store action `setCompander`, `CompanderCard.tsx`.

## 5. Real-time safety & "nothing distorts"
- `process()`: no alloc/lock/IO; scratch pre-sized in `prepare`; params read once via the snapshot. Disabled → bit-exact identity early return. Denormal-flush on band buffers (the IIR crossovers + envelope can decay into denormals → CPU spikes; use the `room.rs` `flush` trick on band outputs / envelope).
- Output clamp `(-4.0, 4.0)` in-stage as a hard guard; the existing −0.3 dBFS master limiter after `Gain` is the final ceiling. Makeup gain is bounded.

## 6. Parameters (global, applied to all 10 bands)
`enabled`, `threshold_db`, `ratio` (≥1), `knee_db` (≥0), `attack_ms`, `release_ms`, `makeup_db` (post-gain), `gate_db` (noise-gate threshold), `expander_ratio` (≥1). (Pre-gain folded into makeup/omitted — YAGNI.) UI: enable toggle + 8 sliders with sensible ranges, plus a couple of macro presets ("Night mode" = high ratio/low threshold; "Punch" = expander-leaning) if cheap.

## 7. Testing (mirrors existing stages)
- Biquad: `set_lowpass`/`set_highpass` at 0 dB pass band content; LP attenuates HF, HP attenuates LF (steady-state checks).
- Crossover: LP+HP sum reconstructs the input to within tolerance (LR4 power-complementary / magnitude-flat at crossover); a band-limited tone lands in the right band.
- BandCompressor: below threshold = ~unity; a loud sustained tone above threshold is reduced toward `threshold + (over/ratio)`; gate/expander reduces very quiet input; output finite.
- Compander stage: `disabled_is_identity` (bit-exact); flat (ratio 1, gate −∞-ish) ≈ identity after settle; sustained loud input stays bounded; a 10-band sum of a flat compressor ≈ input (reconstruction). 
- Gates: `cargo test -p hm-dsp` / `-p hm-core`, `cargo clippy --all-targets -- -D warnings`, `pnpm tsc --noEmit`.

## 8. Build order (tasks)
1. `CompanderState` (hm-core) + `EngineState` + TS mirror.
2. `Biquad::set_lowpass`/`set_highpass` + tests.
3. `compander.rs` — `LrCrossover` + `BandCompressor` + `Compander` stage + tests (the core).
4. Insert into `ProcessChain::standard_with_ir` + chain test.
5. Engine `set_compander` + Tauri `engine_set_compander` + register.
6. TS IPC + store `setCompander` + `CompanderCard.tsx` + render in EnhancerView.

## 9. Non-goals (YAGNI)
Per-band individual UI controls; adjustable crossover frequencies in the UI (fixed to the mobile defaults); lookahead; sidechain; spectrum-overlay band visualization (a flat band readout is fine). These can come later if asked.

## 10. Risks
- LR4 reconstruction error / phase at crossovers → covered by the reconstruction test; LR4 is specifically chosen because it's magnitude-flat at crossover.
- Per-sample dB-domain math cost across 10 bands × N frames → acceptable (mobile runs it on phones); envelope/gain-computer are cheap scalar ops. Disabled = zero cost.
- Denormals in the IIR/envelope tail → flush.
