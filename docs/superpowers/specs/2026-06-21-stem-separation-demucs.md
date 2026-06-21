# Stem Separation (Demucs ML) — desktop

Status: foundation done (2026-06-21)

VirtualDJ-style stems: isolate **Vocals / Drums / Bass / Other** from any track,
with a fader panel (volume + mute/solo). Engine = **Demucs v4 (htdemucs)** —
clean ML stems (~9 dB SDR). Separation is **offline** (separate once, ~20-40 s
per song on CPU, cache the 4 stems); mixing is **real-time** (instant faders).

## Architecture

```
track ──decode──▶ temp WAV ──▶ hm-demucs sidecar (Demucs/ggml) ──▶ 4 stem WAVs (cached)
                                                                        │ decode
                                                                        ▼
   faders ──gains──▶ StemPlaybackSource (hm-audio)  ◀── 4 stem buffers
                          │ mixes vocals·gv + drums·gd + bass·gb + other·go
                          ▼  (then the normal DSP chain: EQ, surround, …)
                       output
```

### ✅ Done + tested
- **`hm-audio::stems`** — `StemPlaybackSource` (an `AudioSource`: 4 interleaved-
  stereo stems + live, **smoothed** per-stem gains → mixed output; drops into the
  engine like file playback) + `StemGains` (lock-free `AtomicU32` f32 gains,
  shared UI↔audio-thread). 2 unit tests (mute-via-gain, shortest-stem playback).
- **`hm-audio::engine`** — `play_stems([Vec<f32>;4])`, `set_stem_gain(stem, gain)`,
  `stem_gains()`. The shared `StemGains` lets faders hit the playing source live.
- **`hm-stems`** — `Demucs` orchestrator: `available()`, `cached(input)`,
  `separate(input_wav, on_progress) -> StemPaths` (runs the sidecar, parses
  `progress=`, caches per source by name+size+mtime). 2 tests, clippy clean.

### ⬜ To build
1. **Sidecar `hm-demucs`** (the only un-verifiable-here piece — needs the user's
   build + the model, like the phone iroh lib):
   - Build from [`sevagh/demucs.cpp`](https://github.com/sevagh/demucs.cpp)
     (ggml/Eigen, CPU) with a thin `main` conforming to the contract:
     `hm-demucs --model <dir> --input <wav> --out <dir>` → prints
     `progress=<0..1>`, writes `vocals/drums/bass/other.wav`.
   - Model: htdemucs 4-source ggml f16 (~81 MB), downloaded on first use into the
     app data dir (or bundled as a Tauri `externalBin` resource).
   - `scripts/build_demucs.sh` (clone + cmake + fetch model) — analogous to
     `hype/scripts/build_remote_android.sh`.
2. **Tauri commands** (`src-tauri/src/commands/stems.rs`):
   - `stems_status(trackPath)` → `{ available, separated }` (sidecar+model present;
     cached?).
   - `stems_separate(trackPath)` `#[command(async)]` → decode track (`hm-audio`)
     → write temp WAV → `Demucs::separate` with progress → emit
     `stems:progress {value}` events → decode the 4 stem WAVs → `engine.play_stems`.
   - `stems_set_gain(stem, gain)`, `stems_gains()`.
   - Manage a `Demucs` (sidecar/model/cache paths) in `lib.rs` setup.
3. **Frontend** — a **Stems panel** (`src/features/stems/StemsPanel.tsx`): a
   "Separate this track" button (→ progress bar driven by `stems:progress`), then
   4 vertical faders (Vocals/Drums/Bass/Other) each with mute + solo, à la
   VirtualDJ. ipc: `stemsStatus/Separate/SetGain` + `onStemsProgress`.

## Notes / decisions
- demucs.cpp (native ggml) over Python demucs (no PyTorch dep) and over ONNX-in-
  Rust (htdemucs is hybrid time+freq — the sidecar encapsulates the full STFT/
  overlap-add pipeline; reimplementing it in `ort` is error-prone + unverifiable).
- Offline-separate-then-mix (not real-time inference): matches how stem DJ
  software actually works; real-time ML (HS-TasNet, ~5 dB) is a worse-quality
  stretch goal.
- Gains are smoothed (~5 ms glide) to avoid fader zipper noise.
- Prior art: the mobile `hype` app ships a DSP (Mid/Side + Butterworth) separator
  (~3-5 dB) — this desktop feature is the ML upgrade.
