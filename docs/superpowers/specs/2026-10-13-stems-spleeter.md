# Stems: Spleeter (Vocals / Drums / Instruments) — desktop

Replaces the previous stem engine, whose weights were licensed for research use
only. Deezer's Spleeter code and pretrained models are MIT; this is a port of the
mobile app's engine (`hype/android/app/src/main/cpp/stems`).

- **Engine** (`crates/hm-stems`): periodic-Hann STFT 4096/1024, 3072 front pad,
  four fp16 nets (ONNX Runtime, CPU, EXTENDED graph opt, no arena/mem pattern),
  soft ratio masks over all four nets, "average" band extension above 11 kHz,
  overlap-add. Work is split into fixed ~11.8 s chunks. `parity/` builds the
  mobile C++ code as a reference; `tests/real_model.rs` checks the port against it.
- **Mixer** (`hm-audio/src/stems.rs`): `out = gV·V + gD·D + gI·(M − V − D)`,
  evaluated as `M + Σ(g−1)·stem` so unity gains return the decode bit for bit.
  Only V and D are stored; where a chunk isn't separated yet the original plays.
- **Pipeline** (`src-tauri/src/commands/stems.rs`): decode → 44.1 kHz → separate
  the playhead … +30 s → swap in at the playhead → worker fills the rest (nearest
  the listener first, follows seeks) → cache (int16 WAV pair) when whole. Works
  for local files and YouTube Music streams (256 KB ranged download, m4a decode).
- **Install**: first use downloads ONNX Runtime 1.22 and the four nets (~79 MB) into
  `<app data>/stems`, SHA-256-pinned; `scripts/get_stems_model.sh` does the same
  for developers.
