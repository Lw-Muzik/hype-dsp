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

Two parts are needed:

1. **The driver + installer** — a signed package that registers the virtual
   device. This is a separate code-signing / packaging effort (it can't be built
   or tested off-Windows).
2. **The WASAPI capture→DSP→render loop** — outlined in
   `system_eq_windows.rs::WindowsSystemEq::start` (the `TODO(windows-driver)`
   plan), to be wired against the installed driver on a real Windows box.

Until both land, `system_eq_windows::available()` reports whether the bundled
device is present (false today) and `start()` returns a clear "install the
HypeMuzik virtual audio device" error rather than shipping untested real-time FFI.

## Frontend

Settings → **System-wide audio** card. `system_audio_available` gates the toggle
per-OS; `player_play_system_audio` / `stop_system_audio` start/stop it. Because the
Linux/Windows pipeline runs out-of-band (not through the engine's play state), the
card tracks its own on/off state.
