#!/usr/bin/env bash
# Build the C++ reference harness against the MOBILE stems sources and write
# reference vocals/drums for a clip. Usage:
#   MOBILE_STEMS_SRC=~/me/COTE/hype/android/app/src/main/cpp/stems \
#   ORT_DIR=/path/to/onnxruntime-osx-arm64-1.22.0 \
#   parity/run.sh MODEL_DIR input.f32 OUT_DIR [CHUNK...]     (default chunks: 0 1)
# input.f32 is interleaved stereo float32 at 44.1 kHz (see parity/make_clip.sh).
# Then: HM_STEMS_MODEL_DIR=... HM_STEMS_PARITY_DIR=OUT_DIR HM_STEMS_PARITY_INPUT=input.f32 \
#       ORT_DYLIB_PATH=$ORT_DIR/lib/libonnxruntime.dylib \
#       cargo test -p hm-stems --release -- --ignored parity
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
: "${MOBILE_STEMS_SRC:?path to hype/android/app/src/main/cpp/stems}"
: "${ORT_DIR:?onnxruntime release folder with include/ and lib/}"
model_dir="$1"; input="$2"; out="$3"; shift 3
chunks=("$@"); [ ${#chunks[@]} -gt 0 ] || chunks=(0 1)
mkdir -p "$out"
bin="$out/harness"
clang++ -std=c++17 -O2 -Wall -Wno-unused-parameter \
  -I"$MOBILE_STEMS_SRC" -I"$ORT_DIR/include" \
  "$here/harness.cpp" "$MOBILE_STEMS_SRC/stft.cpp" "$MOBILE_STEMS_SRC/spleeter_onnx.cpp" \
  -L"$ORT_DIR/lib" -lonnxruntime -Wl,-rpath,"$ORT_DIR/lib" -o "$bin"
"$bin" "$model_dir" "$input" "$out" "${chunks[@]}"
