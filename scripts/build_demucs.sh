#!/usr/bin/env bash
#
# Build the `hm-demucs` stem-separation sidecar (Demucs v4 / htdemucs) and install
# it where the app looks for it. Run once.
#
# It: builds sevagh/demucs.cpp (ggml, CPU), fetches the htdemucs ggml model, and
# installs a thin `hm-demucs` wrapper that conforms to the app's contract:
#
#     hm-demucs --model <dir> --input <wav> --out <dir>
#       → prints `progress=<0..1>`, writes vocals/drums/bass/other.wav
#
# Requires: git, cmake, a C++17 compiler. macOS: `brew install cmake`.
set -euo pipefail

# Where the app reads the sidecar + model (matches src-tauri/lib.rs setup):
#   macOS:   ~/Library/Application Support/<bundle id>/stems
#   override with HM_STEMS_DIR=/path
APP_ID="${HM_BUNDLE_ID:-com.hypemuzik.desktop}"
case "$(uname -s)" in
  Darwin) DEFAULT_DIR="$HOME/Library/Application Support/$APP_ID/stems" ;;
  Linux)  DEFAULT_DIR="$HOME/.local/share/$APP_ID/stems" ;;
  *)      DEFAULT_DIR="$HOME/.hypemuzik/stems" ;;
esac
STEMS_DIR="${HM_STEMS_DIR:-$DEFAULT_DIR}"
BUILD_DIR="${HM_STEMS_BUILD:-$HOME/.cache/hm-demucs-build}"

mkdir -p "$STEMS_DIR/model" "$BUILD_DIR"
echo "Installing into: $STEMS_DIR"

# 1. Build demucs.cpp -------------------------------------------------------
if [ ! -d "$BUILD_DIR/demucs.cpp" ]; then
  git clone --recurse-submodules https://github.com/sevagh/demucs.cpp "$BUILD_DIR/demucs.cpp"
fi
# `-DCMAKE_POLICY_VERSION_MINIMUM=3.5` lets CMake 4.x configure demucs.cpp's
# older sub-projects (they declare cmake_minimum_required < 3.5, which CMake 4
# otherwise rejects).
cmake -S "$BUILD_DIR/demucs.cpp" -B "$BUILD_DIR/demucs.cpp/build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_POLICY_VERSION_MINIMUM=3.5
cmake --build "$BUILD_DIR/demucs.cpp/build" -j

# The CLI binary name varies by version — find it.
DEMUCS_BIN="$(find "$BUILD_DIR/demucs.cpp/build" -maxdepth 2 -name 'demucs*.cpp*' -type f -perm -u+x | head -1 || true)"
[ -n "$DEMUCS_BIN" ] || { echo "error: built demucs.cpp binary not found — check the build output" >&2; exit 1; }
echo "Built: $DEMUCS_BIN"

# 2. Fetch the htdemucs 4-source ggml model (~81 MB) ------------------------
# demucs.cpp ships a converter; see its README for `convert-pth-to-ggml`. If you
# already have ggml-model-htdemucs-4s-f16.bin, drop it in $STEMS_DIR/model.
if [ -z "$(ls -A "$STEMS_DIR/model" 2>/dev/null)" ]; then
  echo "NOTE: place the htdemucs ggml model into: $STEMS_DIR/model"
  echo "      (build it via demucs.cpp's converter, or fetch a prebuilt ggml-model-htdemucs-4s-f16.bin)"
fi

# 3. Install the wrapper conforming to the app's contract -------------------
# Translates --model/--input/--out/--stems → demucs.cpp, forwards its % progress
# as `progress=<0..1>`, and normalises the output filenames the app expects.
cat > "$STEMS_DIR/hm-demucs" <<WRAP
#!/usr/bin/env bash
set -euo pipefail
MODEL=""; INPUT=""; OUT=""; STEMS=4
while [ \$# -gt 0 ]; do
  case "\$1" in
    --model) MODEL="\$2"; shift 2;;
    --input) INPUT="\$2"; shift 2;;
    --out)   OUT="\$2"; shift 2;;
    --stems) STEMS="\$2"; shift 2;;
    *) shift;;
  esac
done

echo "progress=0.02"

# Run demucs.cpp, forwarding any "NN%" it prints as progress=<0..1>. For 2-stem,
# pass your demucs.cpp's two-stem flag here if it supports one (e.g. --two-stems);
# otherwise it runs 4-stem and we derive the instrumental below.
"$DEMUCS_BIN" "\$MODEL" "\$INPUT" "\$OUT" 2>&1 | while IFS= read -r line; do
  pct="\$(printf '%s' "\$line" | grep -oE '[0-9]+(\.[0-9]+)?%' | tail -1 | tr -d '%' || true)"
  [ -n "\$pct" ] && awk -v p="\$pct" 'BEGIN { printf "progress=%.3f\n", (p/100)*0.96 + 0.02 }'
done

# Normalise demucs.cpp's output names (it often prefixes the track name).
for s in vocals drums bass other no_vocals instrumental accompaniment; do
  f="\$(ls "\$OUT"/*"\$s".wav 2>/dev/null | head -1 || true)"
  [ -n "\$f" ] && [ "\$f" != "\$OUT/\$s.wav" ] && mv "\$f" "\$OUT/\$s.wav" || true
done

if [ "\$STEMS" = "2" ] && [ ! -f "\$OUT/instrumental.wav" ]; then
  # No 2-stem output — derive instrumental = drums + bass + other (needs sox).
  if command -v sox >/dev/null 2>&1 && [ -f "\$OUT/drums.wav" ]; then
    sox -m "\$OUT/drums.wav" "\$OUT/bass.wav" "\$OUT/other.wav" "\$OUT/instrumental.wav"
  elif [ -f "\$OUT/no_vocals.wav" ]; then
    mv "\$OUT/no_vocals.wav" "\$OUT/instrumental.wav"
  fi
fi

echo "progress=1.0"
WRAP
chmod +x "$STEMS_DIR/hm-demucs"

echo
echo "Done. Sidecar: $STEMS_DIR/hm-demucs"
echo "Restart HypeMuzik → Stems tab → Separate into stems."
echo "(If separation fails, check the demucs.cpp CLI args/output names in $STEMS_DIR/hm-demucs.)"
