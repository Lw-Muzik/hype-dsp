# HypeMuzik virtual audio driver — package drop folder

This folder is **bundled into the app** (see `tauri.conf.json` →
`bundle.resources` → `drivers/HypeMuzikAudio/*`) and is where the **signed**
virtual-audio driver package goes — the `.inf`, `.sys`, and Microsoft-signed
`.cat`:

```
*.inf
*.sys
*.cat   ← Microsoft attestation/WHQL countersignature
```

**Filenames don't matter** — the app discovers the `.inf` by glob
(`win_driver::find_driver_inf`) and the NSIS hook does the same, so the package is
dropped **exactly as the signed upstream produced it** (renaming a signed `.inf`
would break its `.cat`). Detection keys off the device **friendly name**, which
must contain **`HypeMuzik`**.

It cannot be built or signed on macOS/Linux. Produce it on Windows — either run
the **`Windows Audio Driver` GitHub Actions workflow**
(`.github/workflows/windows-driver.yml`, the one-click path) or follow
[`../../../docs/windows-driver.md`](../../../docs/windows-driver.md) by hand:

1. Build the driver (base: the MIT-licensed
   [`VirtualDrivers/Virtual-Audio-Driver`](https://github.com/VirtualDrivers/Virtual-Audio-Driver),
   or Microsoft's SYSVAD sample), with the device **friendly name containing
   `HypeMuzik`** so `system_eq_windows`/`win_driver` detect it.
2. Sign it (EV certificate + Microsoft Partner Center **attestation signing**).
3. Copy the files here and rebuild the installer (`pnpm tauri build`).

The app degrades gracefully until then: `system_audio_status` reports
`driverInstalled: false`, the Settings card shows **Install audio driver**, and
the installer's `NSIS_HOOK_POSTINSTALL` no-ops.

> Do not commit unsigned/test driver binaries. Keep only this README in version
> control; CI/release tooling injects the signed package.
