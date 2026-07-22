# System-wide equalization

"System-wide EQ" means every app's audio is captured, run through HypeMuzik's DSP
chain (EQ + bass + spatializer + surround + room + limiter), and the **original
is silenced** so you only hear the processed result. Plain loopback/monitor
*capture* is not enough — it grabs audio that's already playing, so replaying the
EQ'd copy would double the sound. True system-wide EQ requires *intercept-and-
replace*, which works differently on each OS.

All three platforms funnel into the same `hm_dsp::ProcessChain` with the engine's
live parameters (`AudioEngine::state_handle()`), so the EQ/effects are identical.

## macOS — Core Audio process taps (shipping)

`crates/hm-audio/src/system_tap.rs`. A **muted** global process tap captures every
app except HypeMuzik and silences their direct output; an aggregate device feeds
the tap into the chain, and the processed mix is rendered to the real device.

- Requires macOS 14.4+, `NSAudioCaptureUsageDescription`, and a code-signed build
  for the audio-capture permission to persist.
- Enabled via `engine.play_system_tap()`.

## Linux — PulseAudio / PipeWire virtual sink (shipping)

`crates/hm-audio/src/system_eq_linux.rs`. The portable Linux approach *re-routes*
rather than taps:

1. create a **null sink** (`module-null-sink`) and make it the **default output**,
   so every app renders into it (existing streams are moved over);
2. capture its `.monitor` with `parec`;
3. run the samples through `ProcessChain` (live params, in a worker thread);
4. play the result to the **real** output device with `pacat`.

The originals go to the null sink, never the speakers — no doubling. This is the
same model as EasyEffects' "process all outputs". On stop it restores the previous
default sink and unloads the null sink.

- Uses the ubiquitous `pactl` / `parec` / `pacat` CLIs, so it needs **no extra
  crates** and works on both PipeWire (pulse layer) and classic PulseAudio.
- No driver, no admin. `hm_audio::system_eq_linux::available()` checks `pactl info`.
- Enabled via `engine.start_system_eq()`, stopped via `engine.stop_system_eq()`.

## Windows — bundled virtual audio device (in progress)

`crates/hm-audio/src/system_eq_windows.rs`. Windows has **no pure user-space way**
to intercept-and-replace system audio (WASAPI loopback can't silence the original).
The chosen approach mirrors VB-Cable / FxSound: ship a **virtual audio output
device**, make it the default so apps render into it, then this process
loopback-captures it → `ProcessChain` → renders to the real device. Same
re-routing model as Linux/macOS.

Status — **the app side is fully implemented**; only the signed driver binary
remains (a Windows-only build+sign step). See **[`windows-driver.md`](windows-driver.md)**
for the turnkey checklist. In this repo:

1. **The WASAPI capture→DSP→render loop** — `system_eq_windows.rs`. Now with a
   **startup handshake** so `WindowsSystemEq::start` reports real pipeline failures
   to the UI instead of returning `Ok` the moment the worker spawns (the old cause
   of the toggle showing a phantom "running" state).
2. **Driver lifecycle** — `win_driver.rs`: `install_driver` (UAC-elevated
   `pnputil`) + `routing_device_available`. Tauri: `system_audio_status` /
   `system_audio_install_driver`. UI shows **Install audio driver** then
   **Enable/Restart/Stop**. Installed at setup time by `installer-hooks.nsh`.
3. **The driver + installer package** — the signed `.inf`/`.sys`/`.cat`. Built and
   signed on Windows (EV cert + Microsoft attestation), dropped into
   `src-tauri/drivers/HypeMuzikAudio/`. Cannot be built off-Windows.
4. **One-click interim setup (shipping today)** — builds without the signed
   driver (`driverBundled: false`) offer **Set up system-wide EQ** instead:
   `commands/cable.rs` downloads VB-CABLE from VB-Audio's official server,
   verifies its pinned SHA-256, silently installs it (`-i -h`, one UAC prompt),
   waits for the device, then the UI auto-enables. Routing candidates are
   matched in priority order — `"HypeMuzik"` first, then `"CABLE Input"`
   (`system_eq_windows::routing_device_names`) — so the branded driver
   automatically supersedes the cable once it ships. See the licensing note in
   [`windows-driver.md`](windows-driver.md).

The pipeline can also be validated against any other virtual cable by setting
`HM_SYSTEM_EQ_DEVICE` to a substring of that device's friendly name (the env
override replaces the whole candidate list). Until a routing device is present,
`system_audio_status` reports `driverInstalled: false` and the UI offers the
setup/install action rather than a phantom running state.

## Frontend

Settings → **System-wide audio** card. `system_audio_available` gates the toggle
per-OS; `player_play_system_audio` / `stop_system_audio` start/stop it. Because the
Linux/Windows pipeline runs out-of-band (not through the engine's play state), the
card tracks its own on/off state.
