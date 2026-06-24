# Content-Adaptive Bass (adaptive gain / anti-overload) — Plan

> REQUIRED SUB-SKILL: superpowers:subagent-driven-development.

**Goal:** Upgrade the static low-shelf `bass_boost.rs` with an optional **adaptive-gain** mode: detect existing low-band energy and scale the boost DOWN when bass is already strong (anti-overload/anti-mud), full boost when bass is weak. Backward-compatible — a new `adaptive` toggle; off = today's exact behavior. Closes JamesDSP "content-adaptive bass" gap (#6).

**Approach (no biquad re-tuning, zipper-free):** keep the shelf at the full `amount` (fixed coeffs). Add a per-channel low-band envelope follower; compute `adapt_factor ∈ [floor,1]` that falls as the envelope rises; blend `out = dry + adapt_factor*(shelved − dry)` (+ existing harmonics). `adaptive=false` ⇒ `adapt_factor=1` ⇒ identical to current.

## Global Constraints
- RT-safety: `process()` allocation-free (envelope/lp state pre-sized in prepare). Disabled → bit-exact identity. Bounded output.
- Backward compat: `adaptive=false` must be byte-for-byte the current behavior (the existing `disabled_is_identity` + `enabled_boosts_low_frequencies` tests must still pass unchanged).
- `BassBoostState.adaptive` serde camelCase, mirrored in TS. Default `adaptive=false`.
- The `engine_set_bass` command/store/ipc use positional args (`enabled, amount, harmonics`) — add `adaptive` as a 4th positional bool consistently across all layers.
- Gates per crate: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `pnpm tsc --noEmit`.

---

## Task 1: `BassBoostState.adaptive` + plumbing (state, engine, command, ipc, store)
**Files:** `crates/hm-core/src/types.rs`, `crates/hm-audio/src/engine.rs`, `src-tauri/src/commands/engine.rs`, `src/lib/types.ts`, `src/lib/ipc.ts`, `src/stores/engine.ts`.

- [ ] **hm-core**: add `pub adaptive: bool` to `BassBoostState` (after `harmonics`); update its `Default` (`adaptive: false`). Update the test helper/`disabled_is_identity` if it constructs `BassBoostState { ... }` literally (add `adaptive: false`). TS `src/lib/types.ts`: add `adaptive: boolean;` to `BassBoostState`.
- [ ] **engine.rs** `set_bass`: change signature to `set_bass(&self, enabled: bool, amount: f32, harmonics: bool, adaptive: bool)` and set `s.bass = BassBoostState { enabled, amount, harmonics, adaptive }`.
- [ ] **commands/engine.rs** `engine_set_bass`: add `adaptive: bool` param; pass to `engine.set_bass(enabled, amount, harmonics, adaptive)`.
- [ ] **ipc.ts** `engineSetBass`: add `adaptive: boolean` param; `invoke("engine_set_bass", { enabled, amount, harmonics, adaptive })`.
- [ ] **engine.ts** store `setBass`: add `adaptive` param; type `setBass: (enabled, amount, harmonics, adaptive) => void`; impl `set bass: { enabled, amount, harmonics, adaptive }` + `engineSetBass(enabled, amount, harmonics, adaptive)`. Update the default-state literal (`bass: { ..., adaptive: false }`).
- [ ] Gates: `cargo test -p hm-core`, `cargo build -p hypemuzik`, `cargo test -p hm-audio`, clippy clean on touched crates, `pnpm tsc --noEmit` clean. Commit `feat(core): add adaptive flag to BassBoostState + plumbing`.

---

## Task 2: Adaptive-gain DSP in `bass_boost.rs`
**Files:** `crates/hm-dsp/src/bass_boost.rs`.
**Consumes:** `params.bass.adaptive`.

Implement (extend the existing `BassBoost`):
- New per-channel state (sized in `reconfigure`/`prepare`): a low-band envelope follower. Reuse a one-pole lowpass (e.g. ~120 Hz, can reuse the harmonic `lp_coeff` style or a dedicated coeff) to isolate the bass, then a one-pole envelope on its absolute value with attack/release coefficients (e.g. attack ~10 ms, release ~150 ms). Store `env: Vec<f32>` per channel + the coeffs.
- `adapt_factor(env) -> f32`: a monotonic decreasing map from envelope level to a gain factor in `[FLOOR, 1.0]` (e.g. FLOOR=0.25). Below a threshold `T_LO` → 1.0 (full boost); above `T_HI` → FLOOR; linear in between. Pick sensible constants (e.g. T_LO=0.05, T_HI=0.4 of full-scale) and DOCUMENT them.
- In `process`, per sample per channel:
  - `dry = x`; `shelved = shelf.process_sample(x)` (as now).
  - if `adaptive`: update the low-band env from `x`; `factor = adapt_factor(env)`; else `factor = 1.0`.
  - `boosted = dry + factor * (shelved - dry)`  (factor=1 ⇒ boosted==shelved ⇒ current behavior).
  - add the existing harmonics term to `boosted` exactly as today (harmonics independent of adaptive).
  - `*sample = boosted` (optionally clamp to a safe bound).
- `set_params`: read `params.bass.adaptive` (cheap bool; no re-tune needed since the shelf stays at full amount). Keep the existing change-guarded `amount` re-tune.

### Tests (add; keep the 2 existing tests passing UNCHANGED)
- existing `disabled_is_identity` + `enabled_boosts_low_frequencies` still pass (adaptive defaults false → unchanged path).
- `adaptive_false_matches_static`: enabled, amount=6, adaptive=false vs the static path → identical output on a test signal (proves backward-compat).
- `adaptive_reduces_boost_on_loud_bass`: adaptive=true, amount=6; feed a SUSTAINED strong low tone (e.g. 60 Hz at high amplitude); after the envelope settles, the steady boost (output/input ratio at the low tone) is LESS than the static (adaptive=false) boost on the same input. (Anti-overload behavior.)
- `adaptive_full_boost_on_quiet_bass`: adaptive=true; a quiet low tone gets ≈the full static boost (factor≈1).
- `stays_bounded`: hostile input bounded.
- Confirm `process` allocation-free.

- [ ] TDD → implement → pass → `cargo clippy -p hm-dsp --all-targets -- -D warnings` → commit `feat(dsp): content-adaptive (anti-overload) bass gain`.

---

## Task 3: UI — "Adaptive" toggle on the bass control
**Files:** `src/features/enhancer/EnhancerView.tsx` (the bass card/section).
- [ ] Find the bass section in EnhancerView (uses `setBass`, `bass.amount`, `harmonics`). Add an "Adaptive" toggle (a `<Switch>` like the harmonics toggle) bound to `bass.adaptive`, calling `setBass(bass.enabled, bass.amount, bass.harmonics, nextAdaptive)`. Update the existing `setBass(...)` calls in that section to pass `bass.adaptive` as the new 4th arg (so toggling amount/harmonics preserves adaptive). Label it "Adaptive (anti-overload)" or similar with the harmonics toggle's styling.
- [ ] `pnpm tsc --noEmit` clean. Commit `feat(ui): adaptive bass toggle`.

---

## Final
- `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings` (4 crates), `pnpm tsc --noEmit` green. Whole-branch review.

## Notes
- The adaptive constants (thresholds/floor/attack/release) can't be ear-tuned here — pick musically-sane defaults, keep the behavior monotonic + bounded, and the tests assert the *relationship* (loud→less, quiet→full) not exact dB.
- `adaptive=false` is the default and the exact current behavior — zero regression risk for existing users.
