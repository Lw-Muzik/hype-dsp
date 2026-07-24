# Windows APO System-Wide EQ Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Windows a **free** (no code-signing cert, no Microsoft Partner/attestation account, no card) system-wide EQ by shipping our own white-label **Audio Processing Object (APO)** that runs the app's `hm_dsp::ProcessChain` inside `audiodg.exe`, installed/attached with one UAC prompt + one reboot.

**Architecture:** A pure-Rust APO `cdylib` (`hm-apo`) implements the Windows APO COM interfaces via the `windows` crate; its real-time `APOProcess` calls `hm_dsp::ProcessChain` directly (same workspace — no FFI). Live EQ/effect params flow from the app to the APO across the process boundary via a **named shared-memory seqlock** carrying a `repr(C)` param snapshot. A Rust installer/attacher (`commands/apo_setup.rs`) side-loads the APO by writing its COM registration + `DisableProtectedAudioDG=1` + the default render endpoint's `FxProperties`, saving and chaining the displaced child APO. A device-change watcher + repair-on-launch re-attach after default-device changes and Windows-update wipes. A backend selector prefers a bundled signed driver → our APO → "offer setup," so a future signed driver auto-supersedes the APO with nothing to undo.

**Tech Stack:** Rust, the `windows` crate (`Win32_Media_Audio`, `Win32_System_Com`, `Win32_System_Registry`, `Win32_System_Memory`), `hm-dsp`/`hm-core`, Tauri 2, `cargo-xwin` for macOS→Windows compile checks.

## Global Constraints

- **Zero cost to ship and to the user:** no EV cert, no Partner Center/attestation, no card, no per-seat license. Copy verbatim into any cost-related decision.
- **White-label:** the end user must never see or sign up for a third-party product. No bundled Equalizer APO, no VB-CABLE in this path.
- **Reuse the one DSP:** all processing goes through `hm_dsp::ProcessChain::standard(rate, channels)` + `set_params(&EngineState)` + `process(&mut [f32], channels)` — never a second DSP implementation.
- **RT contract inside `audiodg.exe`:** `APOProcess` must never allocate, lock, block, log, or panic. Wrap the DSP call in `std::panic::catch_unwind`; bounds-check every buffer. A fault here kills ALL system audio.
- **Windows-only, and not buildable on the macOS/CI-Linux hosts:** `hm-apo` and the APO command code compile only for `*-windows-msvc`. Use `cargo xwin` for compile checks from macOS; **runtime validation is on a real Windows box only.**
- **`DisableProtectedAudioDG=1` is global + Microsoft-unsupported:** it disables the APO signature check system-wide and may degrade DRM/"protected audio" playback. Surface this to the user at install time; provide a clean uninstall that restores it.
- **One reboot** on install/attach (audio-service restart is unreliable on Win11 — offer it as a best-effort fast path, but prompt for reboot).
- **Repair-on-launch:** every launch, verify the endpoint registration still exists (Windows Update wipes it) and re-write it if gone.
- **Mirror the existing one-click pattern:** the installer/attacher follows `src-tauri/src/commands/cable.rs` (single elevation, phase events `system-eq-setup-phase`, honest status).
- **Coexist, never co-run:** the APO backend and the virtual-device backend (`system_eq_windows.rs`) are mutually exclusive; the selector runs exactly one.

---

## File Structure

- `crates/hm-apo/Cargo.toml` — new `cdylib` crate manifest (Windows-target deps only).
- `crates/hm-apo/src/lib.rs` — DLL exports (`DllGetClassObject`, `DllCanUnloadNow`, `DllRegisterServer`, `DllUnregisterServer`), the class factory, module wiring.
- `crates/hm-apo/src/apo.rs` — the APO COM object: `IAudioProcessingObject`, `IAudioProcessingObjectConfiguration`, `IAudioProcessingObjectRT`, `IAudioSystemEffects2`; RT `APOProcess` → `ProcessChain`.
- `crates/hm-apo/src/guids.rs` — our stable CLSID + registry path constants (shared with the installer via a copy in `hm-core`, see below).
- `crates/hm-core/src/apo_ipc.rs` — cross-process param IPC: `EngineParamsPod` (`repr(C)`), `SeqlockWriter`, `SeqlockReader` over a named file mapping. Host-unit-tested (the seqlock + POD round-trip are platform-agnostic; only the mapping handle is `#[cfg(windows)]`).
- `crates/hm-core/src/apo_ids.rs` — the shared constants (CLSID string, mapping name, registry subkeys) used by BOTH `hm-apo` and the installer, so they can never drift.
- `crates/hm-audio/src/system_eq_windows_apo.rs` — app-side APO backend: writes live params to the seqlock, flips the active flag on start/stop.
- `crates/hm-audio/src/system_eq_windows.rs` — MODIFY: add a backend selector (`WindowsBackend`) choosing signed-driver vs APO.
- `src-tauri/src/commands/apo_setup.rs` — install/attach/repair/uninstall commands + `IMMNotificationClient` device watcher + repair-on-launch, with host-testable pure registry-plan helpers.
- `src-tauri/src/commands/engine.rs` — MODIFY: `SystemAudioStatus` gains `apo_installed`; `system_audio_status` reports it.
- `src-tauri/src/lib.rs` — MODIFY: register new commands + call `apo_repair_on_launch` under `#[cfg(windows)]`.
- `src/lib/ipc.ts`, `src/features/settings/systemAudioCard.ts`, `SettingsView.tsx` — MODIFY: APO install/enable/repair affordance + status field.
- `.github/workflows/release.yml`, `src-tauri/tauri.conf.json` — MODIFY: build `hm_apo.dll` for the Windows target and bundle it as a resource.
- `docs/system-eq.md`, `docs/windows-apo.md` — the APO backend docs + the manual Windows validation checklist.

---

## Task 1: Shared APO identity constants (`hm-core`)

**Files:**
- Create: `crates/hm-core/src/apo_ids.rs`
- Modify: `crates/hm-core/src/lib.rs` (add `pub mod apo_ids;`)
- Test: inline `#[cfg(test)]` in `apo_ids.rs`

**Interfaces:**
- Produces: `hm_core::apo_ids::{CLSID_STR, CLSID_GUID, MAPPING_NAME, CLSID_REGKEY, APO_REGKEY, DISABLE_PROTECTED_AUDIO_DG_KEY, DISABLE_PROTECTED_AUDIO_DG_VALUE, FX_PROPERTIES_PKEY}` — all `&'static str`/`GUID` used by both the DLL and the installer so registration and lookup can never disagree.

- [ ] **Step 1: Write the failing test**

```rust
// crates/hm-core/src/apo_ids.rs
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clsid_string_and_guid_agree() {
        // The braced string form must parse to the same 128-bit GUID the DLL uses.
        assert_eq!(CLSID_STR, "{7B1C4A20-9D3E-4E8A-9F2C-11AA22BB33CC}");
        assert_eq!(CLSID_GUID.to_u128(), 0x7B1C4A20_9D3E_4E8A_9F2C_11AA22BB33CC);
    }
    #[test]
    fn registry_paths_are_hklm_relative_and_stable() {
        assert_eq!(CLSID_REGKEY, "SOFTWARE\\Classes\\CLSID\\{7B1C4A20-9D3E-4E8A-9F2C-11AA22BB33CC}");
        assert!(APO_REGKEY.ends_with(CLSID_STR));
        assert_eq!(
            DISABLE_PROTECTED_AUDIO_DG_KEY,
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Audio"
        );
        assert_eq!(FX_PROPERTIES_PKEY, "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p hm-core apo_ids`
Expected: FAIL (`apo_ids` module not found).

- [ ] **Step 3: Write the constants**

```rust
// crates/hm-core/src/apo_ids.rs
//! Identity constants for the HypeMuzik APO, shared verbatim by the APO DLL
//! (`hm-apo`) and the installer (`commands/apo_setup.rs`) so registration and
//! detection can never drift. GUID generated once for this product — never
//! regenerate (a changed CLSID orphans installed registrations).

/// Our APO's class id, braced-string form (registry) — GENERATE ONCE, then frozen.
pub const CLSID_STR: &str = "{7B1C4A20-9D3E-4E8A-9F2C-11AA22BB33CC}";

/// Same id as a 128-bit constant for COM (`GUID::from_u128`).
pub const CLSID_GUID: Guid = Guid(0x7B1C4A20_9D3E_4E8A_9F2C_11AA22BB33CC);

/// Minimal platform-agnostic GUID newtype so `hm-core` needn't depend on the
/// `windows` crate; `hm-apo` converts it to `windows_core::GUID`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Guid(pub u128);
impl Guid {
    pub const fn to_u128(self) -> u128 { self.0 }
}

/// Named file mapping carrying the live `EngineParamsPod` seqlock (Task 2).
pub const MAPPING_NAME: &str = "Local\\HypeMuzikApoParams";

/// `HKLM` COM registration key for the CLSID.
pub const CLSID_REGKEY: &str =
    "SOFTWARE\\Classes\\CLSID\\{7B1C4A20-9D3E-4E8A-9F2C-11AA22BB33CC}";

/// `HKLM` AudioEngine APO catalog entry.
pub const APO_REGKEY: &str =
    "SOFTWARE\\Classes\\AudioEngine\\AudioProcessingObjects\\{7B1C4A20-9D3E-4E8A-9F2C-11AA22BB33CC}";

/// The global switch that lets `audiodg.exe` load unsigned APOs.
pub const DISABLE_PROTECTED_AUDIO_DG_KEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Audio";
pub const DISABLE_PROTECTED_AUDIO_DG_VALUE: &str = "DisableProtectedAudioDG";

/// The endpoint FxProperties PKEY container; the APO CLSID is written into the
/// SFX/EFX (or SFX/MFX) pid slots under
/// `MMDevices\Audio\Render\{endpoint}\FxProperties`.
pub const FX_PROPERTIES_PKEY: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d}";
```

- [ ] **Step 4: Wire the module + run the test**

Add `pub mod apo_ids;` to `crates/hm-core/src/lib.rs`. Run: `cargo test -p hm-core apo_ids` — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/hm-core/src/apo_ids.rs crates/hm-core/src/lib.rs
git commit -m "feat(apo): shared APO identity constants in hm-core"
```

---

## Task 2: Cross-process param IPC — `EngineParamsPod` + seqlock (`hm-core`)

**Files:**
- Create: `crates/hm-core/src/apo_ipc.rs`
- Modify: `crates/hm-core/src/lib.rs` (add `pub mod apo_ipc;`)
- Test: inline `#[cfg(test)]` in `apo_ipc.rs`

**Interfaces:**
- Consumes: `EngineState` (existing) to build a pod; `hm_core::apo_ids::MAPPING_NAME`.
- Produces:
  - `EngineParamsPod` — `#[repr(C)]`, `Copy`, all fields the DSP needs (power flag, master_volume, active flag, eq enabled/pre_gain/bands, and every other numeric field `ProcessChain::set_params` reads). `EngineParamsPod::from_state(&EngineState) -> Self`.
  - `apply_pod(pod: &EngineParamsPod, out: &mut EngineState)` — writes the pod's values back into a reusable `EngineState` (so the APO can call the existing `set_params`).
  - `SeqlockCell` — a `#[repr(C)]` `{ version: AtomicU32, _pad, payload: EngineParamsPod }` living inside the mapping.
  - `write_seqlock(cell: &SeqlockCell, pod: &EngineParamsPod)` / `read_seqlock(cell: &SeqlockCell) -> Option<EngineParamsPod>` — the odd/even version protocol; reader retries on torn/odd reads and returns `None` only if never-written.
  - `#[cfg(windows)] SharedMapping::{create_writer, open_reader}(name) -> io::Result<SharedMapping>` with `cell(&self) -> &SeqlockCell` (CreateFileMapping/MapViewOfFile).

- [ ] **Step 1: Write the failing tests (pure, host-runnable)**

```rust
// crates/hm-core/src/apo_ipc.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_roundtrips_through_engine_state() {
        let mut st = EngineState::default();
        st.power = true;
        st.master_volume = 0.75;
        st.eq.enabled = true;
        st.eq.pre_gain = -3.0;
        st.eq.bands[0] = 6.0;
        let pod = EngineParamsPod::from_state(&st, /*active=*/ true);
        let mut back = EngineState::default();
        apply_pod(&pod, &mut back);
        assert!(back.power);
        assert!((back.master_volume - 0.75).abs() < 1e-6);
        assert!(back.eq.enabled);
        assert!((back.eq.pre_gain + 3.0).abs() < 1e-6);
        assert!((back.eq.bands[0] - 6.0).abs() < 1e-6);
    }

    #[test]
    fn seqlock_reads_last_consistent_write() {
        let cell = SeqlockCell::zeroed();
        let a = EngineParamsPod::from_state(&EngineState::default(), true);
        write_seqlock(&cell, &a);
        let got = read_seqlock(&cell).expect("written once");
        assert_eq!(got.active, 1);
        assert_eq!(got.version_even(), true); // version left even after a write
    }

    #[test]
    fn seqlock_none_before_first_write() {
        let cell = SeqlockCell::zeroed();
        assert!(read_seqlock(&cell).is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p hm-core apo_ipc`
Expected: FAIL (module not found).

- [ ] **Step 3: Implement the POD + seqlock (pure) and the Windows mapping (cfg)**

Write `EngineParamsPod` as `#[repr(C)] #[derive(Clone, Copy)]` mirroring every field `ProcessChain::set_params` consumes (read `crates/hm-dsp/src/lib.rs::set_params` and `crates/hm-core` `EngineState` to enumerate them — EQ bands array, pre_gain, enabled; bass; spatializer; surround; room; limiter; master_volume; power; plus an `active: u32` gate). `from_state` copies fields out; `apply_pod` copies them into a reusable `EngineState` (leave non-DSP/heap fields at default). `SeqlockCell` holds `version: AtomicU32` + payload. `write_seqlock`: `v=load; store(v|1, Release)` (odd = writing), copy payload, `store((v|1)+1, Release)` (even = done). `read_seqlock`: read version (Acquire); if 0 → `None`; if odd → retry a bounded number of times; copy payload; re-read version; if unchanged and even → `Some`, else retry. `#[cfg(windows)] SharedMapping` uses `CreateFileMappingW`/`OpenFileMappingW` + `MapViewOfFile` sized to `size_of::<SeqlockCell>()`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p hm-core apo_ipc` — Expected: PASS (3 tests).

- [ ] **Step 5: cross-compile the Windows mapping path**

Run: `cargo xwin check -p hm-core --target x86_64-pc-windows-msvc`
Expected: builds (validates the `#[cfg(windows)] SharedMapping` against the `windows` crate).

- [ ] **Step 6: Commit**

```bash
git add crates/hm-core/src/apo_ipc.rs crates/hm-core/src/lib.rs
git commit -m "feat(apo): repr(C) param POD + named-mapping seqlock IPC"
```

---

## Task 3: The APO DLL crate skeleton + class factory (`hm-apo`)

**Files:**
- Create: `crates/hm-apo/Cargo.toml`, `crates/hm-apo/src/lib.rs`, `crates/hm-apo/src/guids.rs`
- Modify: root `Cargo.toml` workspace `members`
- Test: `cargo xwin` compile only (a `cdylib` with COM exports isn't host-unit-testable).

**Interfaces:**
- Consumes: `hm_core::apo_ids`.
- Produces: the DLL exports `DllGetClassObject`, `DllCanUnloadNow`, `DllRegisterServer`, `DllUnregisterServer`; a `ClassFactory` that creates the `HypeMuzikApo` (Task 4).

- [ ] **Step 1: Manifest**

```toml
# crates/hm-apo/Cargo.toml
[package]
name = "hm-apo"
version.workspace = true
edition.workspace = true
description = "HypeMuzik system-wide EQ Audio Processing Object (Windows)."

[lib]
crate-type = ["cdylib"]

[target.'cfg(windows)'.dependencies]
hm-core = { workspace = true }
hm-dsp = { workspace = true }
windows-core = "0.61"
windows = { version = "0.61", features = [
    "Win32_Foundation",
    "Win32_Media_Audio",
    "Win32_Media_Audio_Apo",
    "Win32_System_Com",
    "Win32_System_Com_StructuredStorage",
    "Win32_System_Registry",
    "Win32_System_SystemServices",
] }
```

- [ ] **Step 2: GUID conversion + exports skeleton**

```rust
// crates/hm-apo/src/guids.rs
use windows_core::GUID;
pub const CLSID_HYPEMUZIK_APO: GUID = GUID::from_u128(hm_core::apo_ids::CLSID_GUID.to_u128());
```

Write `lib.rs` with `#![cfg(windows)]`, the standard COM in-proc exports delegating to a `ClassFactory` (use `windows` crate `IClassFactory` via `#[implement]`), `DllCanUnloadNow` backed by an `AtomicI32` ref count, and `DllRegisterServer`/`DllUnregisterServer` writing/removing the `CLSID_REGKEY` `InprocServer32` (path = this module's own path via `GetModuleFileNameW`, `ThreadingModel=Both`) and the `APO_REGKEY` catalog entry. (Registration is duplicated by the installer for robustness, but a self-registering DLL is the conventional COM contract.)

- [ ] **Step 3: Add to the workspace + cross-compile**

Add `crates/hm-apo` to the workspace `members`. Run: `cargo xwin build -p hm-apo --target x86_64-pc-windows-msvc`
Expected: produces `hm_apo.dll` (compile-only validation).

- [ ] **Step 4: Commit**

```bash
git add crates/hm-apo Cargo.toml
git commit -m "feat(apo): hm-apo cdylib skeleton with COM exports + class factory"
```

---

## Task 4: The APO object + RT `APOProcess` → `ProcessChain` (`hm-apo`)

**Files:**
- Create: `crates/hm-apo/src/apo.rs`
- Modify: `crates/hm-apo/src/lib.rs` (wire the factory to `HypeMuzikApo`)
- Test: `cargo xwin` compile + on-Windows manual validation (Task 12).

**Interfaces:**
- Consumes: `hm_core::apo_ipc::{SharedMapping, read_seqlock, apply_pod, EngineParamsPod}`, `hm_dsp::ProcessChain`.
- Produces: `HypeMuzikApo` implementing `IAudioProcessingObject`, `IAudioProcessingObjectConfiguration`, `IAudioProcessingObjectRT`, `IAudioSystemEffects2`.

- [ ] **Step 1: Implement the COM object**

Using the `windows` crate `#[implement]` for the four interfaces (`windows::Win32::Media::Audio::Apo::*`):
- `Initialize` — no-op / stash nothing that needs the reg.
- `IsInputFormatSupported` / `IsOutputFormatSupported` — accept **32-bit float, ≤ engine channels**, mirror EqAPO: return the suggested format.
- `LockForProcess(input, output)` — read the negotiated frame rate/channels; build `ProcessChain::standard(rate, channels)` once; `open_reader(MAPPING_NAME)` for params; allocate the reusable `EngineState` + last-seen version. All allocation happens HERE, not in `APOProcess`.
- `IAudioProcessingObjectRT::APOProcess(connections)` — the RT path:
  ```
  // pseudocode contract — see Global Constraints
  let _ = std::panic::catch_unwind(|| {
      let n = frames * channels;                    // from APO_CONNECTION_PROPERTY
      let buf: &mut [f32] = /* map connection buffer, bounds-checked */;
      if let Some(pod) = self.reader.try_pod() {     // lock-free seqlock read
          if pod.version != self.last_version {      // rebuild EngineState only on change
              apply_pod(&pod, &mut self.state);
              self.chain.set_params(&self.state);
              self.last_version = pod.version;
          }
          if pod.active == 1 && self.state.power {
              self.chain.process(buf, channels);     // in place
          } else if (self.state.master_volume - 1.0).abs() > f32::EPSILON {
              for s in buf { *s *= self.state.master_volume; }
          }
      }
      // active==0 / no params yet → pass through untouched (never silence)
  });
  ```
  Never allocate, lock, log, or panic; `catch_unwind` guarantees a DSP fault degrades to pass-through instead of crashing `audiodg.exe`.
- `UnlockForProcess` — drop the chain/reader.

- [ ] **Step 2: Wire the factory + cross-compile**

Point the Task 3 `ClassFactory::CreateInstance` at `HypeMuzikApo`. Run: `cargo xwin build -p hm-apo --target x86_64-pc-windows-msvc` — Expected: builds.

- [ ] **Step 3: Commit**

```bash
git add crates/hm-apo/src/apo.rs crates/hm-apo/src/lib.rs
git commit -m "feat(apo): APO object with RT APOProcess calling ProcessChain (pass-through-safe)"
```

---

## Task 5: Registry-plan pure helpers (`commands/apo_setup.rs`)

**Files:**
- Create: `src-tauri/src/commands/apo_setup.rs`
- Modify: `src-tauri/src/commands/mod.rs` (`pub mod apo_setup;`)
- Test: inline `#[cfg(test)]`.

**Interfaces:**
- Consumes: `hm_core::apo_ids::*`.
- Produces (pure, host-testable):
  - `enum ApoSlot { SfxEfx, SfxMfx }` and `choose_slot(is_composite: bool) -> ApoSlot` (composite/Bluetooth ⇒ `SfxMfx`, else `SfxEfx`).
  - `fx_value_names(slot: ApoSlot) -> [&'static str; 2]` — the two `"{PKEY},pid"` value names to write (e.g. `,5`/`,7` for SFX/EFX; `,5`/`,6` for SFX/MFX).
  - `endpoint_fx_key(endpoint_guid: &str) -> String` — `MMDevices\Audio\Render\{guid}\FxProperties`.
  - `ApoRegistryPlan { com_keys, fx_key, fx_values, disable_dg }` and `plan_install(endpoint_guid, slot, dll_path) -> ApoRegistryPlan` — the full, ordered list of registry writes (data only; no IO).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composite_endpoints_use_sfx_mfx() {
        assert_eq!(choose_slot(true), ApoSlot::SfxMfx);
        assert_eq!(choose_slot(false), ApoSlot::SfxEfx);
    }
    #[test]
    fn fx_value_names_match_slot() {
        let pk = hm_core::apo_ids::FX_PROPERTIES_PKEY;
        assert_eq!(fx_value_names(ApoSlot::SfxEfx), [format!("{pk},5").as_str(), format!("{pk},7").as_str()]);
    }
    #[test]
    fn endpoint_key_is_render_scoped() {
        let k = endpoint_fx_key("{abc}");
        assert!(k.starts_with("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\MMDevices\\Audio\\Render\\{abc}"));
        assert!(k.ends_with("\\FxProperties"));
    }
    #[test]
    fn install_plan_sets_the_dg_flag_and_fx_values() {
        let plan = plan_install("{abc}", ApoSlot::SfxEfx, "C:/hm_apo.dll");
        assert!(plan.disable_dg);
        assert_eq!(plan.fx_values.len(), 2);
        assert!(plan.com_keys.iter().any(|k| k.contains("InprocServer32")));
    }
}
```

- [ ] **Step 2: Run to verify fail** — `cargo test -p hypemuzik apo_setup` — Expected: FAIL.
- [ ] **Step 3: Implement the pure helpers** (data only; note `fx_value_names` returns owned `String`s if needed for the PKEY interpolation — adjust the test to `String` accordingly).
- [ ] **Step 4: Run to verify pass** — `cargo test -p hypemuzik apo_setup` — Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/apo_setup.rs src-tauri/src/commands/mod.rs
git commit -m "feat(apo): pure registry-plan helpers for APO install"
```

---

## Task 6: The elevated installer/attacher (`commands/apo_setup.rs`, Windows)

**Files:**
- Modify: `src-tauri/src/commands/apo_setup.rs` (add `#[cfg(windows)] mod imp` + the Tauri command)
- Modify: `src-tauri/src/lib.rs` (register `apo_install`, `apo_uninstall`, `apo_repair`)
- Test: `cargo xwin` compile + Windows manual validation.

**Interfaces:**
- Consumes: the Task 5 plan helpers; `hm_core::apo_ids`; `tauri::AppHandle` (resolve the bundled `hm_apo.dll` via `resource_dir()`).
- Produces (Tauri commands, cross-platform signatures like `cable.rs`): `apo_install(app) -> Result<ApoInstallOutcome, IpcError>` (`Installed` | `NeedsReboot`), `apo_uninstall(app)`, `apo_repair(app)`.

- [ ] **Step 1: Implement the install flow**

`imp::install(app)`:
1. Resolve `resource_dir()/apo/hm_apo.dll`; copy to a fixed per-machine dir (e.g. `%ProgramFiles%\HypeMuzik\apo\hm_apo.dll`) — requires elevation.
2. Because HKLM writes + ProgramFiles copy need admin and Tauri commands run unelevated, do the whole mutation in **one elevated helper invocation**: build a `.reg`-equivalent sequence and run it via `powershell Start-Process -Verb RunAs -Wait` calling our own `--apo-apply <planfile>` subcommand (mirror `cable.rs`'s elevation), or `reg add` batch. Single UAC prompt.
3. The elevated step: enumerate the **default render endpoint** (`IMMDeviceEnumerator::GetDefaultAudioEndpoint(eRender, eConsole)`), read its endpoint GUID + composite flag (`{b3f8fa53-0004-438e-9003-51a46e139bfc},41`), `choose_slot`, save the **displaced child APO** GUIDs currently in the FX slots to `HKLM\SOFTWARE\HypeMuzik\ApoChild`, then apply `plan_install`: write `InprocServer32`, the `AudioProcessingObjects` entry, `DisableProtectedAudioDG=1`, and the FX slot values = our CLSID.
4. Emit `system-eq-setup-phase` (`checking`/`installing`/`ready`). Return `NeedsReboot` (offer a best-effort `net stop audiosrv && net start audiosrv` as the fast path but default to a reboot prompt).

`imp::uninstall(app)`: restore the saved child APO GUIDs into the FX slots, remove our CLSID/AudioProcessingObjects/InprocServer32 keys, and (only if we set it) clear `DisableProtectedAudioDG`.

- [ ] **Step 2: cross-compile** — `cargo xwin clippy -p hypemuzik --target x86_64-pc-windows-msvc -- -D warnings` on the touched module (full-app xwin may choke on other native deps — scope to compile-checking this file's arm; document if the projectM/SDL2 deps block it, as in the memory notes).
- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands/apo_setup.rs src-tauri/src/lib.rs
git commit -m "feat(apo): elevated one-UAC installer/attacher + uninstall"
```

---

## Task 7: Device-follow watcher + repair-on-launch

**Files:**
- Modify: `src-tauri/src/commands/apo_setup.rs` (add `#[cfg(windows)] apo_repair_on_launch`, `IMMNotificationClient`)
- Modify: `src-tauri/src/lib.rs` (call it in `setup()` under `#[cfg(windows)]`)
- Test: pure helper `needs_reattach(current_endpoint, attached_endpoint) -> bool` unit-tested; watcher validated on Windows.

**Interfaces:**
- Produces: `apo_repair_on_launch(app)` — if the APO is "installed" (CLSID present) but its FX registration is missing from the current default endpoint (Windows Update wiped it, or the default changed), re-attach it silently (no UAC needed only if HKLM is writable by the process — otherwise emit a `system-eq-setup-phase("repair-needed")` event so the UI offers a one-click repair); `needs_reattach`.

- [ ] **Step 1: Failing test for `needs_reattach`**

```rust
#[test]
fn reattach_when_default_endpoint_changed() {
    assert!(needs_reattach("{new}", Some("{old}")));
    assert!(needs_reattach("{new}", None));
    assert!(!needs_reattach("{same}", Some("{same}")));
}
```

- [ ] **Step 2–4:** run→fail, implement `needs_reattach` + the `IMMNotificationClient` (`OnDefaultDeviceChanged` → schedule re-attach) + launch-time check, run→pass.
- [ ] **Step 5: Commit** — `git commit -m "feat(apo): device-change watcher + repair-on-launch"`

---

## Task 8: App-side APO backend — live param writer (`hm-audio`)

**Files:**
- Create: `crates/hm-audio/src/system_eq_windows_apo.rs`
- Modify: `crates/hm-audio/src/lib.rs`, `crates/hm-audio/src/engine.rs` (`start_system_eq`/`stop_system_eq` select the backend)
- Test: pure `EngineParamsPod::from_state` already covered (Task 2); the writer thread validated on Windows.

**Interfaces:**
- Consumes: `Arc<ArcSwap<EngineState>>` (engine params), `hm_core::apo_ipc::{SharedMapping, write_seqlock, EngineParamsPod}`.
- Produces: `ApoBackend { start(state) -> Result<Self, AudioError>, /* Drop: active=0 */ }` — `create_writer(MAPPING_NAME)`, spawn a low-rate (e.g. 60 Hz) writer thread that snapshots `state.load()` → `EngineParamsPod{active:1}` → `write_seqlock`; on Drop, write `active:0` once so the APO passes through, then stop.

- [ ] **Steps:** create the module; the writer thread is trivial and its correctness is the seqlock (already tested). `cargo xwin check -p hm-audio --target x86_64-pc-windows-msvc`. Commit `feat(apo): app-side live-param writer backend`.

---

## Task 9: Windows backend selector + honest status

**Files:**
- Modify: `crates/hm-audio/src/system_eq_windows.rs` (add `enum WindowsBackend { SignedDriver, Apo, None }` + `select()`), `crates/hm-audio/src/engine.rs` (`start_system_eq` dispatches), `src-tauri/src/commands/engine.rs` (`SystemAudioStatus.apo_installed`)
- Test: pure `select(driver_present, apo_installed) -> WindowsBackend` unit-tested.

**Interfaces:**
- Produces: `select(driver_present: bool, apo_installed: bool) -> WindowsBackend` — priority: `SignedDriver` (bundled virtual driver present) > `Apo` (our CLSID registered) > `None`. `apo_installed()` = the CLSID key exists.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn signed_driver_wins_then_apo_then_none() {
    assert_eq!(select(true, true), WindowsBackend::SignedDriver);
    assert_eq!(select(false, true), WindowsBackend::Apo);
    assert_eq!(select(false, false), WindowsBackend::None);
}
```

- [ ] **Steps 2–4:** implement `select`, route `start_system_eq` to `WindowsSystemEq` (driver) or `ApoBackend` (APO), add `apo_installed` to `SystemAudioStatus`. Run tests.
- [ ] **Step 5: Commit** — `feat(apo): windows backend selector + apo_installed status`.

---

## Task 10: Frontend — install / enable / repair affordance

**Files:**
- Modify: `src/lib/ipc.ts` (`apoInstall`/`apoUninstall`/`apoRepair` + `apo_installed` on the status type), `src/features/settings/systemAudioCard.ts` (affordance logic), `src/features/settings/SettingsView.tsx`
- Test: `src/features/settings/systemAudioCard.test.ts` (extend), `tsc --noEmit`, `vitest`.

**Interfaces:**
- Produces: `systemAudioAffordance` gains an `"install-apo" | "enable" | "repair"` result for Windows: `apo_installed && available` → enable/stop; `apo_installed && !attached` → repair; else → "Set up system-wide EQ" (installs our APO, one UAC + reboot). Copy: never names "APO" or any third party to the user — call it "system-wide EQ".

- [ ] **Step 1: Failing vitest** for the new affordance branches. Steps 2–4: implement, `tsc --noEmit` + `vitest run systemAudioCard`. Step 5: commit `feat(apo): settings affordance for system-wide EQ install/enable/repair`.

---

## Task 11: Build + bundle `hm_apo.dll`

**Files:**
- Modify: `.github/workflows/release.yml` (Windows job builds `hm-apo` for `x86_64-pc-windows-msvc` and stages `hm_apo.dll`), `src-tauri/tauri.conf.json` (`bundle.resources` adds `apo/hm_apo.dll`), `crates/hm-apo` output wiring
- Test: CI build; the installer resolves the bundled DLL (Task 6).

**Interfaces:** the release Windows job runs `cargo build -p hm-apo --release --target x86_64-pc-windows-msvc` and copies `hm_apo.dll` into `src-tauri/apo/` before `tauri build`; `tauri.conf.json` bundles `apo/hm_apo.dll` so `resource_dir()/apo/hm_apo.dll` exists at runtime.

- [ ] **Steps:** add the build+stage step (guarded to the Windows matrix entry), the `bundle.resources` glob, and a `src-tauri/apo/README.md` placeholder so the glob is valid when the DLL isn't staged locally. Commit `build(apo): compile + bundle hm_apo.dll on the windows release job`.

---

## Task 12: Docs + on-Windows validation checklist

**Files:**
- Create: `docs/windows-apo.md`
- Modify: `docs/system-eq.md` (Windows section: free APO backend is now the default; signed-driver = future premium; VB-CABLE demoted to BYO)

- [ ] **Step 1: Write the validation checklist** (the only real runtime test — a Windows box):
  1. `cargo build -p hm-apo --release --target x86_64-pc-windows-msvc` on Windows produces `hm_apo.dll`.
  2. App → Settings → "Set up system-wide EQ" → one UAC → reboot.
  3. After reboot: `HKLM\...\Audio\DisableProtectedAudioDG == 1`; the default endpoint's `FxProperties` contains our CLSID; `hm_apo.dll` present in ProgramFiles.
  4. Play audio in another app (browser/Spotify); toggle a +12 dB low-shelf in HypeMuzik → the change is audible on that other app, live.
  5. Toggle system EQ off → other-app audio returns to flat instantly (`active=0` pass-through), no reboot.
  6. **Crash-safety:** confirm audio never dies when toggling rapidly; if `audiodg.exe` ever restarts, the `catch_unwind` guard held.
  7. Change the default output device → EQ re-attaches (watcher) or the card offers Repair.
  8. Windows Update that reinstalls the audio driver → next launch, Repair re-attaches.
  9. **DRM check:** confirm the `DisableProtectedAudioDG` warning is shown at install and note any protected-audio app that misbehaves.
  10. Uninstall → child APO restored, our keys gone, EQ off.
- [ ] **Step 2: Commit** — `docs(apo): windows APO backend docs + on-device validation checklist`.

---

## Self-Review notes

- **Spec coverage:** every research recommendation maps to a task — own-APO DLL (T3/T4), side-load registry technique + `DisableProtectedAudioDG` (T1/T5/T6), shared-memory live params (T2/T8), device-follow + repair-on-launch (T7), backend selector + status (T9), one-click installer mirroring `cable.rs` (T6), FE (T10), bundle (T11), validation (T12).
- **Untestable-on-host reality:** T3/T4/T6/T7(watcher)/T11 are compile-checked via `cargo xwin` and runtime-validated only on Windows (T12). All genuinely pure logic (identity constants, POD/seqlock, registry plan, slot choice, backend select, `needs_reattach`, FE affordance) has real host unit tests.
- **Risk register carried in Global Constraints:** `audiodg` crash (mitigated by `catch_unwind` + pass-through-on-fault), `DisableProtectedAudioDG` global/DRM caveat (user-warned + reversible), Windows-update wipe (repair-on-launch), one reboot, per-endpoint attach, unsupported-technique exposure.
