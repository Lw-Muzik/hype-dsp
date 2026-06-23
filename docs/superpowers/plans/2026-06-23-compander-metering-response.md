# Compander v2 — Gain-Reduction Metering + Faster Response — Plan

> REQUIRED SUB-SKILL: superpowers:subagent-driven-development.

**Goal:** Make the 10-band compander production-grade — (1) a per-band gain-reduction (GR) meter so users can see it working, and (2) a faster, attack-driven response (+ modest lookahead) so it isn't sluggish.

**Branch:** `feat/multiband-compander` (continues the compander work).

## Global Constraints
- RT-safety: `Compander::process` stays allocation-free; GR is published via a lock-free `CompanderMeter` (atomics), written once per block. Lookahead delay rings pre-sized in `prepare`.
- Telemetry mirrors the existing `meters`/`spectrum` pattern EXACTLY: Arc handle created in `AudioEngine::new`, cloned into the control thread, written by the compander on the audio thread, read by `forward_frames` (~60fps) and emitted on `engine:frame` inside `EngineFrame`, consumed via `onEngineFrame`.
- Flat reconstruction must STILL hold (now shifted by the lookahead): with the compander enabled + flat (ratio 1, no gate), output delayed by the lookahead L equals the input (telescoping preserved). Adjust the reconstruction test to compare with the L-sample shift.
- Gates per touched crate: `cargo test -p <crate>`, `cargo clippy --all-targets -- -D warnings`, `pnpm tsc --noEmit`.

---

## Task 1: DSP — faster response (gain ballistics + lookahead) + per-band GR exposure

**Files:** Modify `crates/hm-dsp/src/compander.rs`, `crates/hm-dsp/src/lib.rs` (export `CompanderMeter`); Test in compander.rs.

**Produces:**
- `pub struct CompanderMeter` — 10 `AtomicU32` (per-band GR in dB as bit pattern); `new()`, `store_band(i, gr_db)`, `load() -> [f32; 10]`, `Default`. Lives in compander.rs, re-exported from lib.rs.
- `Compander::with_meter(sample_rate, channels, meter: Arc<CompanderMeter>) -> Self`; `Compander::new(...)` makes a throwaway meter via `Arc::new(CompanderMeter::default())`.
- `Compander::meter() -> Arc<CompanderMeter>` accessor.

**Response changes (in `BandCompressor`):**
1. **Gain ballistics** — replace the fixed `GAIN_SMOOTH=0.005` gain slew with attack/release-driven coefficients: when `gain_db < gain_smoothed_db` (more reduction needed) move with the ATTACK coefficient; otherwise the RELEASE coefficient. Reuse the already-computed `attack_coeff`/`release_coeff`. This makes `attack_ms` actually control clamp speed (the prior fixed slew overrode it → "sluggish").
2. **Lookahead** — add a fixed modest lookahead `LOOKAHEAD_MS = 3.0`. Each `BandCompressor` keeps a per-channel ring buffer of `lookahead_samples` (pre-sized in `prepare`/`init`). Per frame: push the incoming L/R into the ring, compute envelope+gain from the INCOMING (undelayed) sample, but apply the gain to the sample LEAVING the ring (delayed by `lookahead_samples`). When flat (gain≈1) the band output = delayed input, so the chain adds `lookahead_samples` of latency and flat reconstruction = input shifted by L. Keep it allocation-free (ring allocated in prepare).
   - NOTE: all bands use the SAME `lookahead_samples`, so the summed bands stay phase-aligned and reconstruction still telescopes (to a delayed input).
3. **GR exposure** — after computing `gain_smoothed_db` for a band, the `Compander` writes that band's GR (`gain_smoothed_db`, ≤0) to `meter.store_band(i, gr)` once per block (e.g. the last frame's value, or the min/most-reduced over the block). Cheap atomic store; do it per-block, not per-sample.

**Tests:**
- `disabled_is_identity` still bit-exact.
- `flat_compander_reconstructs_input` — now assert `out[L..]` ≈ `in[..len-L]` (RMS error < 2%) where L = lookahead samples; reconstruction still holds, shifted.
- `fast_attack_reduces_faster_than_slow` — two companders, one attack=1ms one attack=100ms, same loud step input; the fast one reaches its target reduction in fewer samples (proves ballistics wired to attack).
- `meter_reports_reduction_under_compression` — under heavy compression at least one band's `meter.load()` is clearly negative (reduction); flat/disabled → ~0.
- `stays_bounded` still holds.
- Confirm `process` has no allocation (rings/meter pre-built).

- [ ] TDD: write tests → fail → implement → pass → `cargo clippy -p hm-dsp --all-targets -- -D warnings` → commit `feat(dsp): compander gain ballistics + lookahead + per-band GR meter`.

---

## Task 2: Telemetry plumbing — meter through engine + EngineFrame + forwarder

**Files:** Modify `crates/hm-dsp/src/lib.rs` (`standard_with_ir`/`standard` signatures), `crates/hm-audio/src/engine.rs`, `crates/hm-core/src/types.rs` (+TS `src/lib/types.ts`), `src-tauri/src/lib.rs` (forward_frames).

**Produces:** `EngineFrame.compander_gr: Option<Vec<f32>>`; `AudioEngine::compander_gr() -> Arc<CompanderMeter>`.

- [ ] **`standard_with_ir`** gains a param `gr_meter: Arc<CompanderMeter>`, passed to `Compander::with_meter(...)`. `standard` creates a throwaway `Arc::new(CompanderMeter::default())` and forwards. Update the convolver/chain tests that call `standard_with_ir` to pass `Arc::new(hm_dsp::CompanderMeter::default())` (or add an `empty_compander_meter()` helper mirroring `empty_ir_slot`). The 2 system-eq callers use `standard` (unchanged signature) — unaffected.
- [ ] **Engine**: create `let compander_gr = Arc::new(CompanderMeter::default());` in `new`, clone into the control thread, pass to `Renderer::new(... , ir_slot, compander_gr.clone())` → into `standard_with_ir`. Add field + `pub fn compander_gr(&self) -> Arc<CompanderMeter>`. Update the 3 `Renderer::new` call sites (1 real + 2 tests) and `Renderer::new` signature to take + forward the meter.
- [ ] **`EngineFrame`** (hm-core): add `pub compander_gr: Option<Vec<f32>>,` (serde camelCase → `companderGr`). Mirror in TS `EngineFrame` interface: `companderGr?: number[] | null`. Update any `EngineFrame { ... }` literals (the idle-settle one in forward_frames and the active one).
- [ ] **forward_frames** (src-tauri): take the `compander_gr` handle (via `engine.compander_gr()` near `engine.meters()`), and in the active emit add `compander_gr: Some(gr.load().to_vec())`; idle-settle emit → `compander_gr: None` (or zeros).
- [ ] Gates: `cargo build -p hypemuzik`, `cargo test -p hm-audio`, `cargo test -p hm-dsp`, clippy clean on touched crates, `pnpm tsc --noEmit`. Commit `feat(audio): publish per-band compander GR to the UI via engine:frame`.

---

## Task 3: UI — 10-bar gain-reduction meter in CompanderCard

**Files:** Modify `src/stores/engine.ts` (consume `companderGr` from the frame), `src/features/enhancer/CompanderCard.tsx`.

- [ ] **Store**: the engine store already updates `meters` from `onEngineFrame`. Add a `companderGr: number[]` slice (default 10 zeros), updated in the same frame handler from `frame.companderGr ?? zeros`. (Match how `meters`/spectrum are stored.)
- [ ] **Card**: above/alongside the sliders, render a compact 10-bar GR meter — read `companderGr` from the store; each bar's height/length maps GR dB (0 = no reduction → empty; e.g. −12 dB → full) using a fixed range (0..−18 dB). Tailwind only, no new dep; dim when `!enabled`. Keep it small and consistent with the card styling. Label "Gain reduction".
- [ ] Gates: `pnpm tsc --noEmit` clean. Commit `feat(ui): per-band gain-reduction meter in CompanderCard`.

---

## Final
- `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings` (hm-core/dsp/audio/hypemuzik), `pnpm tsc --noEmit` — all green.
- Whole-branch review (the metering range a4ec3f3..HEAD).

## Notes
- Lookahead adds ~3ms latency on top of the convolver's; fine for a player. If it complicates reconstruction, the gain-ballistics fix alone already addresses "sluggish" — keep lookahead only if clean.
- GR sign convention: meter stores gain in dB (≤0); the UI shows |GR| as reduction.
