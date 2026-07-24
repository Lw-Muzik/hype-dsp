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

## Linux — dual backend, PipeWire-first (shipping)

`crates/hm-audio/src/system_eq_linux.rs` is a **dispatcher** that picks a backend
at runtime, because Linux has two incompatible audio-server families. It keeps the
`available()` / `LinuxSystemEq::start()` facade the engine calls, and selects:

### PipeWire — native client (primary, `system_eq_pipewire.rs`)

The macOS-parity path: transparent, zero-config, crash-safe, low-latency. On
modern distros (~90–95 % of 2026 desktops) PipeWire's **WirePlumber** owns routing
policy, so the old `pactl set-default-sink` + `move-sink-input` trick is only
*advisory* — apps frequently keep playing to the real device **unprocessed**. That
was the "system EQ does nothing on Linux" bug. EasyEffects avoids it by being a
resident native client, and so do we:

1. a **sink stream** (`media.class = Audio/Sink`, node `hypemuzik_eq`) *is* a
   selectable virtual sink; the graph mixes every app routed to it and delivers the
   audio to our RT process callback, which pushes it into a lock-free ring;
2. an **output stream** (`AUTOCONNECT` to the real default device) pops the ring,
   runs `ProcessChain`, and renders to the device;
3. a **stream mover** — a registry listener — sets `target.object` metadata on
   every `Stream/Output/Audio` node (except our own output and HypeMuzik's own
   playback) so it routes into our sink. `target.object` *is* WirePlumber's routing
   mechanism, so it cooperates instead of fighting; new, existing, and even
   user-pinned streams are all captured and re-captured on each new-node event.

We deliberately **do not switch the default sink** (WirePlumber overrides that).
Because our nodes are owned by our client connection, a crash makes PipeWire
destroy them and WirePlumber re-route apps back to the real device — no
system-wide silence, unlike the null-sink approach.

- Adds the `pipewire` crate (0.10) → build-time `libpipewire-0.3-dev` + `libclang`
  + `pkg-config`; runtime `libpipewire-0.3.so.0` (declared as a `.deb`/`.rpm`
  dependency). Needs libpipewire ≥ 0.3.65, so the Linux **build** runner is
  `ubuntu-24.04` (22.04 ships 0.3.48).
- `available()` = the `$XDG_RUNTIME_DIR/pipewire-0` socket exists.

### classic PulseAudio — virtual sink (fallback, `system_eq_pulse.rs`)

For the shrinking minority on a real PulseAudio daemon (no PipeWire), where the
`module-null-sink` + `parec`/`pacat` model *is* reliable. Repaired from the first
cut so it can't phantom-"run": every `pactl` exit status is checked, the default
switch is **read back and verified**, `available()` also confirms `parec`/`pacat`
exist, and a startup handshake fails loudly if no audio actually moves.

- `available()` = `pactl info` reachable **and** `parec`/`pacat` on `PATH`.
- Known limitation: hardware volume keys change the (null-sink) default, not our
  render level to the real device — use the app's own volume. (PipeWire path is
  unaffected.)

### selection & honest availability

`classify_stack()` is PipeWire-first: on a PipeWire box `pactl` *also* works via
`pipewire-pulse`, so the Pulse probe can be true too, but we must never fall back
to the broken CLI while PipeWire is live. Neither present → honestly unavailable
(no toggle). Enabled via `engine.start_system_eq()`, stopped via
`engine.stop_system_eq()`.

> **Snap note (configured):** `snap/snapcraft.yaml` is set up for PipeWire under
> strict confinement — `base: core24` (matches the ubuntu-24.04 build's glibc and
> ships a modern libpipewire), the `audio-playback`/`audio-record` plugs for the
> host `pipewire-0` socket, and `libpipewire-0.3-0` in `stage-packages` (the
> extracted `.deb` doesn't resolve its `Depends`). Verify on an actual snap build.

### launch-time dependency auto-install

`src-tauri/src/commands/linux_audio_setup.rs` (`auto_setup_on_launch`, wired in
`lib.rs setup()`). Most users need nothing — PipeWire already provides everything
and the `.deb`/`.rpm` `Depends: libpipewire` pulls the runtime lib at install
time. When system EQ *is* unavailable only because a package is missing (e.g.
`pulseaudio-utils` on a PipeWire-less box), it installs it on launch:

- Detects the package manager (`apt`/`dnf`/`pacman`/`zypper`) from `PATH` and runs
  `pkexec <pm> install …` — **one polkit prompt** (Linux can't install silently
  like the Windows VB-CABLE flow; system packages need root).
- Never touches a whole audio server, only the missing user-space tools; no-op
  when nothing's needed.
- **Sandbox (Flatpak/Snap), unknown distro, or a declined/failed prompt** → emits
  `system-eq-setup-manual` with the exact `sudo …` command; the app toasts it and
  copies it to the clipboard. A declined auto-prompt is remembered (a marker in
  the config dir) so it won't re-ask every launch; the Settings action
  (`linux_system_audio_setup`) is a manual retry that ignores the marker.
- Package names come from a fixed allowlist and `pkexec` is invoked without a
  shell — no injection surface.

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
