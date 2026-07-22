# Windows system-wide EQ — the bundled virtual audio driver (Option 2)

System-wide EQ on Windows routes every app's audio through a **virtual output
device**, captures it (WASAPI loopback), runs the DSP chain, and renders the
result to the real output device. For a zero-setup experience — the model Boom3D
and FxSound use — we ship a **bundled, signed virtual audio driver** that provides
that device. This is the premium, "it just works" path that fits the freemium
product: the user installs HypeMuzik and system-wide EQ is simply available.

The **app side is fully implemented** (see "What's done" below). The one part that
**cannot be built or signed off-Windows** is the kernel-mode driver binary and its
Microsoft signature. This document is the turnkey checklist to produce it on
Windows and drop it into the build.

---

## Architecture recap

```
 every app ──▶ [HypeMuzik virtual output device]  (default endpoint)
                          │  WASAPI loopback capture
                          ▼
                 hm_dsp::ProcessChain  (EQ + bass + spatial + room + limiter)
                          │
                          ▼
                 real default output device  (speakers / headphones)
```

- The virtual device is the bundled driver. The pipeline detects it by friendly
  name (must contain **`HypeMuzik`**) and loopback-captures it.
- `IPolicyConfig::SetDefaultEndpoint` makes it the default so apps render into it
  automatically; the previous default is restored on stop.
- Files: `crates/hm-audio/src/system_eq_windows.rs` (capture→DSP→render loop +
  default-endpoint switching) and `crates/hm-audio/src/win_driver.rs` (install /
  status).

## What's done (app side, in this repo)

- **Honest pipeline status.** `WindowsSystemEq::start` now performs a startup
  handshake: the worker reports whether the WASAPI pipeline actually initialized,
  so real failures surface to the UI instead of the toggle showing a phantom
  "running" state. (`system_eq_windows.rs`.)
- **Driver lifecycle.** `win_driver::install_driver` (UAC-elevated `pnputil
  /add-driver … /install`) and `win_driver::routing_device_available`.
- **Tauri commands.** `system_audio_status` (`supported` / `available` /
  `driverInstalled` / `needsDriver`) and `system_audio_install_driver`.
- **UI.** The Settings → *System-wide audio* card shows **Install audio driver**
  when the driver is missing, then **Enable / Restart / Stop** once present.
- **Packaging.** `tauri.conf.json` bundles `drivers/HypeMuzikAudio/*`;
  `installer-hooks.nsh` installs the driver at app-install time (elevated) and
  removes it on uninstall.
- **Config override for testing.** `HM_SYSTEM_EQ_DEVICE` env var points the
  pipeline at *any* installed virtual device (VB-Cable, VoiceMeeter, the open
  Virtual Audio Driver), so the real-time loop can be validated on real hardware
  **before** the signed driver exists.

## What remains (Windows-only, this doc)

1. Build the driver. 2. Sign it. 3. Drop it in `src-tauri/drivers/HypeMuzikAudio/`.

The **`Windows Audio Driver` GitHub Actions workflow**
(`.github/workflows/windows-driver.yml`) automates 1–3: it checks out the driver
source, sets the friendly name, builds (Release|x64), generates + signs the
catalog, and uploads the package as the `hypemuzik-windows-driver` artifact. It is
a **scaffold** — being a Windows-only pipeline it has never run from the dev host,
so the first run will pin the WDK version, the driver solution/output paths, and
the `Inf2Cat` OS list (each marked `VERIFY:` in the workflow). Attestation (step 2,
below) is the one part it cannot fully automate.

---

## 1. Choose & build the driver

Two viable bases (both derive from Microsoft's SYSVAD "Simple Audio Sample"):

| Base | License | Notes |
|------|---------|-------|
| [`VirtualDrivers/Virtual-Audio-Driver`](https://github.com/VirtualDrivers/Virtual-Audio-Driver) | MIT (+ MS-PL sample portions) | Maintained, can be a default render endpoint. Review `THIRD_PARTY_NOTICES.md` for commercial bundling. |
| [Microsoft SYSVAD](https://github.com/microsoft/Windows-driver-samples/tree/main/audio/sysvad) | MIT (MS sample license) | Full control; more work. |

> **Correction (2026-07-22):** an earlier revision called the upstream's releases
> "pre-signed" as a fast path. Verified wrong for shipping: their "(Signed)"
> releases are SignPath Foundation code signing — **not** Microsoft attestation —
> and their README says test-signing mode is required. Kernel drivers on stock
> Win10 1607+/Win11 load only with a Microsoft signature, so step 2 below cannot
> be skipped regardless of base. (The upstream author advertises custom builds
> for commercial use — contact@mikethetech.com — a possible alternative to doing
> the attestation ourselves.)

Requirements:

- **Friendly name must contain `HypeMuzik`** (e.g. "HypeMuzik Virtual Audio").
  This is what `routing_device_name()` matches. Set it in the INF
  (`DeviceName`/`FriendlyName` strings) or rename via the base driver's config.
- Render endpoint, shared-mode, stereo float (matches the capture path; non-stereo
  is down/up-mixed). 48 kHz default is fine.
- Build with the **WDK** (matching Visual Studio + Windows SDK) for `x64` (and
  `arm64` if you ship ARM Windows).

## 2. Sign it (mandatory on Win10 1607+ / Win11)

Kernel-mode drivers won't load unless signed through Microsoft:

1. Obtain an **EV code-signing certificate** (OV is rejected for drivers).
2. Create a **Microsoft Partner Center** (Hardware Dev) account and register the
   EV cert.
3. **Attestation-sign** the driver package (free; automated checks, no full HLK):
   submit the `.cab` to Partner Center → download the Microsoft-countersigned
   package (produces the `.cat`).

See Microsoft's [Attestation signing guide](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/code-signing-attestation).

> Test-signing (`bcdedit /set testsigning on`) is fine for **local dev only** — it
> requires disabling Secure Boot and shows a desktop watermark. Never ship it.

## 3. Bundle it

Copy the signed package into `src-tauri/drivers/HypeMuzikAudio/` **as-is** — keep
the upstream `.inf`/`.sys`/`.cat` filenames (renaming a signed `.inf` invalidates
its `.cat`). The app finds the `.inf` by glob (`win_driver::find_driver_inf`) and
the NSIS hook does the same, so the names are free.

```
src-tauri/drivers/HypeMuzikAudio/
  <whatever>.inf
  <whatever>.sys
  <whatever>.cat
```

Then `pnpm tauri build`. The NSIS installer (`installer-hooks.nsh`) installs it at
setup; the in-app **Install audio driver** button repairs/installs it at runtime.

To ship it from CI: have `release.yml`'s Windows job download the
`hypemuzik-windows-driver` artifact into that folder before `tauri-action` runs
(one `actions/download-artifact` step), so the signed driver lands in the
installer without committing binaries to git.

> Keep signed binaries out of git; inject them in CI/release. The committed README
> placeholder keeps the resource glob valid when the driver isn't present.

---

## The shipped interim path — one-click VB-CABLE setup (no driver yet)

Until the signed driver exists, driverless builds are not dead ends: the
Settings card offers **Set up system-wide EQ** (`src-tauri/src/commands/cable.rs`),
which downloads VB-CABLE from `download.vb-audio.com` (pinned URL + pinned
SHA-256 — the elevated installer never runs unverified), extracts it, runs
`VBCABLE_Setup_x64.exe -i -h` under one UAC prompt, waits for the device, and
auto-enables the EQ. If the device doesn't enumerate, the UI shows an honest
"Restart your PC to finish setup" state.

Detection accepts routing devices in priority order (`system_eq_windows::
routing_device_names`): `"HypeMuzik"` (the bundled driver, once it exists), then
`"CABLE Input"`. Shipping the signed driver therefore *supersedes* the cable
automatically — nothing to migrate or undo.

When VB-Audio publishes a new driver pack, the pinned hash goes stale and setup
fails **closed** with a manual-install fallback; bump `VBCABLE_URL` +
`VBCABLE_SHA256` in `commands/cable.rs` together.

> **Licensing:** VB-CABLE is VB-Audio **donationware**. This flow downloads from
> their official servers on explicit user action only — but distributing it in a
> commercial build should be covered by a licensing/distribution agreement with
> VB-Audio (they offer one; see vb-audio.com → Licensing). Get that agreement
> before a public release that includes this button, or gate the button off.

Power users can still point the pipeline at any other cable manually:

```powershell
# Any installed virtual cable works, e.g. VoiceMeeter:
$env:HM_SYSTEM_EQ_DEVICE = "VoiceMeeter Input"   # replaces the whole candidate list
# launch HypeMuzik, enable system-wide EQ
```

## Freemium notes

- The seamless driver experience is the natural **Pro** hook: gate
  `system_audio_install_driver` / the Enable action behind license state if
  desired (the licensing API already exists — see the Management integration).
- Free tier can still expose the cable/BYO path for power users without diluting
  the premium, zero-setup driver experience.

## References

- Apple-of-Windows equivalents: Boom3D and FxSound both ship a signed virtual
  audio driver; Equalizer APO uses the alternative in-pipeline APO injection.
- [SYSVAD sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/sysvad-virtual-audio-device-driver-sample/),
  [Driver code-signing requirements](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/code-signing-reqs).
