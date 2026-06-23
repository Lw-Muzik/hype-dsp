# Convolver (impulse-response) engine + GraphicEQ-string import — Design

**Date:** 2026-06-23
**Status:** Approved (pending spec review)
**Scope:** Add two JamesDSP-inspired capabilities to HypeMuzik Desktop's DSP chain — a real-time **convolution (impulse-response) engine**, then an **EqualizerAPO GraphicEQ-string import** for the existing 31-band EQ. Convolver first.

## 1. Goals

1. **Convolver:** apply user/bundled impulse responses (real halls, cabinets, speaker/headphone-correction IRs) to the audio stream in real time.
2. **GraphicEQ import:** let users paste/load an EqualizerAPO `GraphicEQ` curve (the universal AutoEQ interchange format) onto the existing graphic EQ, with an automatic, clip-proof preamp.
3. **Non-negotiable performance bar:** the app must stay highly performant with all features on. The audio callback ("the main"/real-time thread) and the UI thread must **never block, allocate, lock, or do I/O** because of these features. Adding the convolver must not make the system "feel slow."
4. **Non-negotiable correctness bar:** output power must be balanced so **nothing distorts** — neither swapping IRs nor importing aggressive EQ curves can clip the output.

## 2. Non-goals (YAGNI for this pass)

- True-stereo 4-channel IR matrix (LL/LR/RL/RR). We support **mono** (same IR both channels) and **stereo** (per-channel IR) only.
- Online AutoEQ database fetch (deferred follow-up; import is paste-text + `.txt` file this pass).
- IR waveform editor / offline IR resampling UI.
- ViPER DDC, LiveProg/EEL, compander, tube saturation (separate future specs from the JamesDSP gap backlog).

## 3. Guiding software principles

- **Single Responsibility / Separation of Concerns.** Three distinct responsibilities, three distinct homes:
  - *Real-time convolution* — pure DSP, no I/O, in `crates/hm-dsp/src/convolver.rs`.
  - *IR acquisition & preparation* (decode, resample, normalize, partition, pre-FFT) — off the audio thread, in the engine/command layer.
  - *State & UI* — `hm-core` types + Tauri commands + a React card.
- **Real-time-safety contract preserved.** The existing `AudioProcessor` trait is unchanged. `prepare`/`set_params`/`process` keep their documented contracts (`process` never allocates/locks/does I/O).
- **Open/Closed.** A new stage is *added* to `ProcessChain::standard()`; no existing stage is modified. The chain abstraction already supports this.
- **Don't-overload-the-main.** All expensive, variable-cost work (file decode, sample-rate conversion, FFT planning, IR partitioning) happens once, off-thread, and is handed to the audio thread by a lock-free atomic pointer swap. The per-block cost on the audio thread is bounded and constant.
- **Follow existing patterns.** Mirror `room.rs` (cached/change-guarded params, denormal flush, `disabled = identity`, output clamp, thorough unit tests) and the `system_eq` `ArcSwap` `state_handle()` handoff already in the codebase.

## 4. Architecture & data flow

```
UI (ConvolverCard.tsx)
  │  invoke engine_convolver_load_ir(path) / engine_set_convolver(state)
  ▼
Tauri command (commands/engine.rs)               ── OFF the audio thread ──
  │  decode WAV (hound) → resample to engine SR → cap length → normalize
  │  → split into uniform partitions → forward-FFT each partition (cached realfft planner)
  ▼  = a ready-to-run PreparedIr (Arc)
ArcSwap<Option<PreparedIr>>  (shared handle, lock-free)
  ▲
  │  load() — a single atomic pointer read, no lock
Convolver stage (hm-dsp) in process()            ── ON the audio thread ──
  │  uniform-partitioned overlap-save FFT multiply-accumulate on pre-planned data
  ▼
ProcessChain:  Headphone → GraphicEq → Bass → Spatializer → Surround3D → Room
               → **Convolver** → Gain → Limiter(−0.3 dBFS brickwall)
```

- `ConvolverState` is added to `EngineState` (`hm-core/types.rs`) and follows the serde/camelCase + `Default` conventions of `RoomState`.
- The `Convolver` stage holds an `ArcSwap<Option<PreparedIr>>` clone. `set_params` updates cheap scalars (enabled, wet/dry, gain) only — never touches the IR. IR changes arrive via the ArcSwap, published by the command thread.

## 5. Convolver DSP design

### 5.1 Algorithm: uniform-partitioned overlap-save FFT convolution
- The IR is split into **K partitions** of size = the engine's audio block size `B`. Each partition is zero-padded to `2B` and forward-FFT'd once at prep time → `K` complex spectra.
- Per audio block: FFT the incoming `2B` window once; for each partition multiply-accumulate `input_spectrum_history[k] · ir_spectrum[k]` into an accumulator; inverse-FFT; take the valid (overlap-save) half as output.
- A ring of the last `K` input spectra is maintained so the cost is **K complex multiply-adds of size `2B` per block — constant and bounded**, independent of where in the IR we are.
- **Latency:** one partition (`B` samples) — matched to the engine's existing block latency; no extra perceptible delay for a player.
- Rejected alternatives (documented for posterity): direct time-domain FIR (O(N·M), stalls on long IRs) and single big-FFT overlap-add (per-block cost/latency scale with IR length → dropouts).

### 5.2 Off-thread preparation pipeline (`PreparedIr`)
Runs entirely in the Tauri command handler thread:
1. **Decode** the IR file with `hound` (WAV/`.irs`; both already-available deps). Reject unsupported gracefully with an `IpcError`.
2. **Resample** to the engine sample rate if needed.
3. **Length cap:** truncate to ≤ `MAX_IR_SECONDS` (default 4 s). If truncated, return that fact so the UI can surface a notice. This bounds CPU (K) and memory.
4. **Normalize** (see §6.1).
5. **Partition + forward-FFT** every partition with a cached `realfft` `RealToComplex` plan (planned once, reused).
6. Wrap in `Arc<PreparedIr>` and publish via `ArcSwap::store`. The audio thread picks it up on its next block with a single lock-free `load()`.

### 5.3 Real-time `process()` contract
- If `!enabled` or no IR loaded or `wet <= 0` → **bit-exact identity early-return** (zero added cost when off).
- No allocation, no lock, no I/O. FFT scratch buffers are pre-sized in `prepare()`.
- **Denormal flush** on the overlap/accumulator state (the `room.rs` `flush()` trick) so convolution tails can't fall into denormal range and spike CPU.
- Mono IR → applied to both channels; stereo IR → per-channel. Channel count handled like `room.rs`.

### 5.4 Performance budget & safeguards (the "highly performant" bar)
- Constant per-block work: `O(K)` size-`2B` complex MACs. `K = ceil(IR_len / B)`, capped via `MAX_IR_SECONDS`.
- FFT plans created once (planner cached), never per block.
- IR prep is off-thread; UI stays responsive; audio never waits on it (atomic swap).
- Stage is skipped entirely when disabled.
- A documented worst-case: 4 s IR @ 48 kHz, `B = 256` → `K ≈ 750` partitions; this is well within real-time budget for a single stereo stream on a modern CPU, and is the *cap*, not the default. Bundled defaults will be far shorter.

## 6. Gain staging — "nothing distorts" (four layers)
1. **IR energy normalization on load:** scale the IR by its L2 norm so the convolved signal's energy ≈ the input's. Swapping a long/loud IR for a short/quiet one keeps perceived level stable.
2. **Wet/dry mix + IR-gain trim (dB):** correction IRs run ~100% wet; reverb IRs partial. User trim for taste.
3. **In-stage output clamp** (±4.0, as `room.rs`) — a hard guard before the output stages.
4. **Existing master Limiter** (`OutputState.ceiling_db = −0.3 dBFS`, look-ahead brickwall) at the chain tail catches any residual transient. The convolver is deliberately placed *before* Gain→Limiter so this safety net always applies.

Combined, the output is mathematically prevented from exceeding the brickwall ceiling.

## 7. State, commands, UI (Convolver)

**`hm-core/types.rs` — `ConvolverState`** (serde camelCase, `Default` = disabled):
- `enabled: bool`
- `wet_dry: f32` (0–1)
- `ir_gain_db: f32`
- `ir_id: Option<String>` (loaded IR identifier/path for UI display)
- `ir_name: Option<String>`, `ir_truncated: bool`, `ir_seconds: f32` (metadata for UI; not read by audio thread)
Added as `pub convolver: ConvolverState` on `EngineState`.

**`commands/engine.rs`** (mirrors `engine_set_room`):
- `engine_convolver_load_ir(engine, path) -> Result<ConvolverIrInfo, IpcError>` — runs the off-thread prep, publishes `PreparedIr`, returns metadata (name, seconds, truncated).
- `engine_set_convolver(engine, state: ConvolverState)` — cheap scalar update.
- Both registered in `src-tauri/src/lib.rs` invoke handler.

**UI — `src/features/enhancer/ConvolverCard.tsx`:** enable toggle, IR picker (file dialog) + bundled-IR `Combobox` (the app's only dropdown component, per house rule — no native `<select>`), wet/dry + gain sliders (shared `<Slider>` **with a width class** to avoid the known zero-width footgun), and a truncation notice. Wired through the Zustand engine store like `RoomCard`.

## 8. GraphicEQ-string import (stage two — no new DSP)

- **Parser:** accept EqualizerAPO `GraphicEQ: f1 g1; f2 g2; …` and the bare AutoEQ `GraphicEQ.txt` payload (same content). Tolerant of whitespace and a leading `GraphicEQ:` label.
- **Interpolation:** log-frequency interpolate the parsed (freq, gain) curve onto the 31 `ISO_CENTERS_HZ` → fill `EqState.bands`. Endpoints clamp to nearest.
- **Auto-preamp (clip-proof):** set `EqState.pre_gain = −max(0, max(bands))`, guaranteeing `max(bands) + pre_gain ≤ 0` — the curve is always net-attenuating, so an imported correction can never add gain or clip. This mirrors AutoEQ's recommended-preamp practice and directly serves the "balanced power" requirement.
- **Where:** lives in `hm-core` (alongside `ISO_CENTERS_HZ`/`BAND_COUNT`/`EqState`, which it operates on) as pure functions `parse_graphic_eq(&str) -> Result<Vec<(f32,f32)>, _>` + `interpolate_to_iso_bands(curve) -> [f32; BAND_COUNT]` + `recommended_preamp(bands) -> f32`; surfaced via a Tauri command and an "Import curve" affordance (paste box + `.txt` file) in the existing EQ card. No audio-thread changes — it just writes `eq.bands`/`pre_gain` through the existing `engine_set_eq` path.

## 9. Testing & verification

Convolver (`hm-dsp` unit tests, mirroring `room.rs`):
- `disabled_is_identity` (bit-exact) and `no_ir_is_identity`.
- Unit-impulse IR (single 1.0 sample) → output equals input (within FP epsilon) = passthrough sanity.
- Known short IR → output matches a direct time-domain reference convolution (null/diff test).
- `stays_bounded_under_sustained_input` after energy normalization.
- Length cap respected (K computed from capped length).

GraphicEQ import:
- Parser round-trips a known string; rejects malformed input.
- Interpolation hits ISO centers exactly when the curve has points there.
- `max(bands) + pre_gain ≤ 0` always holds (property test over random curves).

Gates before "done": `cargo test` (workspace), `cargo clippy` clean, `tsc` clean. Browser/runtime smoke of the cards where feasible (engine commands are runtime-testable; convolution audio output verified via the unit/null tests since on-device audio isn't automatable here).

## 10. Build order

1. `ConvolverState` in `hm-core` + `EngineState` wiring + defaults/tests.
2. `convolver.rs`: partitioned overlap-save engine + `PreparedIr` + tests (TDD: identity, impulse, null-vs-reference, bounded).
3. Off-thread prep pipeline + `ArcSwap` handoff in the engine; Tauri commands; register in `lib.rs`.
4. `ConvolverCard.tsx` + store wiring.
5. GraphicEQ parser/interpolator + auto-preamp + tests.
6. EQ-card import affordance + command.
7. Full gate run (test/clippy/tsc), then verification.

## 11. Risks & mitigations
- **CPU on very long IRs** → `MAX_IR_SECONDS` cap + bounded partition count; documented worst case in §5.4.
- **Audio-thread stalls during IR swap** → off-thread prep + lock-free `ArcSwap`; the audio thread only ever does an atomic pointer read.
- **Distortion from hot IRs / EQ curves** → four-layer gain staging (§6) + auto-preamp (§8); limiter is the final brickwall.
- **Block-size coupling** → partition size derived from the engine's actual block size in `prepare()`; if the engine uses variable block sizes, the overlap-save window is sized to the max block and handles short blocks by zero-fill.
