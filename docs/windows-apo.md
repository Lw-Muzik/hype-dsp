# Windows free system-wide EQ — the APO backend

The zero-cost Windows path to system-wide EQ: **no code-signing certificate, no
Microsoft Partner/attestation account, no card.** We ship our own **Audio
Processing Object (APO)** — a small COM DLL the Windows audio engine loads into
`audiodg.exe` and calls per block to process every app's audio **in place**. This
is the same mechanism Equalizer APO has used for 15 years; the enabler is a single
registry switch that lets the engine load an unsigned APO.

This replaces the previous free interim (bundling VB-CABLE). The signed
virtual-driver path (`system_eq_windows.rs` + `docs/windows-driver.md`) stays as a
future premium option; the backend selector prefers it when present, so shipping a
signed driver later supersedes the APO with nothing to undo.

## Why it's free

APOs normally come from a sound-card driver and are WHQL-signed. Ours never will
be, so — exactly like Equalizer APO — the installer sets:

```
HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Audio\DisableProtectedAudioDG = 1  (DWORD)
```

which disables the APO signature check so `audiodg.exe` loads our unsigned DLL. No
attestation, no EV cert, no Partner Center. **Caveat (surface to the user):** this
is a global, Microsoft-unsupported setting that can degrade DRM/"protected audio"
playback for some apps, and a future Windows build could tighten it. Uninstall
clears it.

## Architecture (what's built)

| Piece | Where | Status |
|---|---|---|
| APO identity (CLSID, registry paths, mapping name) | `hm-core/src/apo_ids.rs` | ✅ host-tested |
| Cross-process live params (`EngineParamsPod` + named-mapping seqlock) | `hm-core/src/apo_ipc.rs` | ✅ host-tested + xwin |
| **The APO DLL** (COM object, RT `APOProcess` → `ProcessChain`, class factory, self-registration) | `crates/hm-apo/` | ✅ compiles for `x86_64-pc-windows-msvc` |
| Backend selector, slot-plan helpers | `hm-audio/src/system_eq_windows_apo.rs` | ✅ host-tested |
| Live-param writer (`ApoBackend`), install probe (`apo_installed`) | same | ✅ xwin-clean |

**How audio flows:** apps → the audio engine's per-endpoint effect chain → our APO
(inside `audiodg.exe`) runs `hm_dsp::ProcessChain` in place → speakers. Params
flow app → `ApoBackend` (60 Hz) → seqlock shared memory → the APO's RT
`APOProcess`, which applies them lock-free and, on any DSP fault, degrades to
pass-through (`catch_unwind`) instead of crashing `audiodg`.

## What remains — and why it needs a Windows box

Everything above compiles (verified via `cargo xwin`) but nothing has run in
`audiodg` yet. The remaining pieces are Windows-develop-and-validate work:

### 1. The elevated installer/attacher (`commands/apo_setup.rs`) — the crux

One UAC prompt (mirror `commands/cable.rs`'s elevation) that:

1. Copies the bundled `hm_apo.dll` to a fixed dir (e.g.
   `%ProgramFiles%\HypeMuzik\apo\hm_apo.dll`).
2. Registers it: call the DLL's `DllRegisterServer` (via `regsvr32` or
   `LoadLibrary`+`GetProcAddress`), or write the keys directly — `CLSID_REGKEY\
   InprocServer32` = the DLL path (`ThreadingModel=Both`) and `APO_REGKEY`
   (`hm_core::apo_ids`).
3. Sets `DisableProtectedAudioDG = 1`.
4. **Attaches to the default render endpoint** — the hard part:
   - Enumerate it: `IMMDeviceEnumerator::GetDefaultAudioEndpoint(eRender, eConsole)`,
     read its endpoint GUID and the composite flag
     (`{b3f8fa53-0004-438e-9003-51a46e139bfc},41`).
   - `system_eq_windows_apo::choose_slot(is_composite)` → SFX/EFX (normal) or
     SFX/MFX (composite/Bluetooth). `fx_value_names(slot)` gives the two
     `"{PKEY},pid"` value names under `endpoint_fx_key(guid)`.
   - **Save the displaced child APO** GUIDs currently in those FX slots (to
     `HKLM\SOFTWARE\HypeMuzik\ApoChild`) so uninstall restores them.
   - **Write our CLSID into those FX values.** ⚠️ These `FxProperties` values are
     **REG_BINARY serialized `PROPVARIANT` property-store blobs**, not plain
     strings — this is the one piece that must be built and verified on Windows
     (get the exact blob format from how Equalizer APO's `DeviceAPOInfo.cpp`
     writes them, or by round-tripping an existing endpoint's value). Use
     `IPropertyStore` on the endpoint (`IMMDevice::OpenPropertyStore(STGM_READWRITE)`)
     rather than hand-serializing the blob — that's the robust path.
5. Prompt a **reboot** (offer `net stop audiosrv && net start audiosrv` as a
   best-effort fast path, but Win11 often needs the reboot).

Uninstall: restore the saved child GUIDs, remove our keys, clear
`DisableProtectedAudioDG`.

### 2. Device-follow + repair (`IMMNotificationClient`)

- Watch `OnDefaultDeviceChanged` → re-attach to the new default endpoint.
- **Repair-on-launch:** every start, if `apo_installed()` but our CLSID isn't in
  the current default endpoint's FX slots (Windows Update wipes it), re-attach.

### 3. Frontend (`SettingsView` + `ipc.ts`)

A Windows branch of the system-audio card: `apo_installed && attached` →
Enable/Stop (drives `ApoBackend` via `start_system_eq`/`stop_system_eq`);
`apo_installed && !attached` → Repair; else → "Set up system-wide EQ" (runs the
installer, one UAC + reboot). Never name "APO" or any third party to the user.
`system_audio_status` gains an `apo_installed` field.

### 4. Bundle the DLL

- Windows release job: `cargo build -p hm-apo --release --target
  x86_64-pc-windows-msvc`, stage `hm_apo.dll` into `src-tauri/apo/`.
- `tauri.conf.json` `bundle.resources`: add `apo/hm_apo.dll` → the installer
  resolves it via `resource_dir()/apo/hm_apo.dll`.

## On-device validation checklist (a real Windows box — the only true test)

1. `cargo build -p hm-apo --release --target x86_64-pc-windows-msvc` → `hm_apo.dll`.
2. Settings → "Set up system-wide EQ" → one UAC → reboot.
3. After reboot: `DisableProtectedAudioDG == 1`; the default endpoint's
   `FxProperties` holds our CLSID; the DLL is in ProgramFiles.
4. Play audio in another app (browser/Spotify); toggle a +12 dB low-shelf in
   HypeMuzik → the change is audible on that other app, live.
5. Toggle system EQ off → other-app audio returns to flat instantly (`active=0`
   pass-through), no reboot.
6. **Crash-safety:** rapid toggling never kills system audio; if `audiodg.exe`
   ever restarts, the `catch_unwind` guard held.
7. Change the default output device → EQ re-attaches, or the card offers Repair.
8. A Windows Update that reinstalls the audio driver → next launch, Repair
   re-attaches.
9. **DRM check:** confirm the `DisableProtectedAudioDG` warning shows at install;
   note any protected-audio app that misbehaves.
10. Uninstall → child APO restored, our keys gone, EQ off.

## Build/verify from a non-Windows host

`cargo xwin build -p hm-apo --target x86_64-pc-windows-msvc` compiles the DLL;
`cargo xwin check -p hm-audio --target x86_64-pc-windows-msvc` checks the backend.
Pure logic is host-tested: `cargo test -p hm-core apo_ipc apo_ids`,
`cargo test -p hm-audio --lib system_eq_windows_apo`. Runtime behaviour in
`audiodg` cannot be validated off Windows.
