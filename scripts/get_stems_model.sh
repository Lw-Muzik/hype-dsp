#!/usr/bin/env bash
#
# Install the stem-separation engine for HypeMuzik Desktop:
#   1. ONNX Runtime (the CoreML-enabled Apple build) -> libonnxruntime.dylib
#   2. htdemucs_ft ONNX models (4 per-stem specialists) -> model/*.onnx
#
# The models are StemSplit's parity-verified ONNX export of htdemucs_ft, so there
# is NO PyTorch and no local export -- just a download. The app loads the dylib at
# runtime (ORT_DYLIB_PATH) and separates the playing track in-process on the
# Neural Engine. Run once.
#
# Requires: curl, tar.  ~1.3 GB download (set HM_STEMS_FP16=1 for the ~640 MB
# fp16-weight variant).
set -euo pipefail

# ORT version must match the API version `ort` 2.0.0-rc.10 requests (ORT_API_VERSION
# 22 → ONNX Runtime >= 1.22). Override with HM_ORT_VER.
ORT_VER="${HM_ORT_VER:-1.22.0}"
HF_REPO="StemSplitio/htdemucs-ft-onnx"

APP_ID="${HM_BUNDLE_ID:-com.hypemuzik.desktop}"
case "$(uname -s)" in
  Darwin) DEFAULT_DIR="$HOME/Library/Application Support/$APP_ID/stems" ;;
  Linux)  DEFAULT_DIR="$HOME/.local/share/$APP_ID/stems" ;;
  *)      DEFAULT_DIR="$HOME/.hypemuzik/stems" ;;
esac
STEMS_DIR="${HM_STEMS_DIR:-$DEFAULT_DIR}"
BUILD_DIR="${HM_STEMS_BUILD:-$HOME/.cache/hm-stems-build}"
mkdir -p "$STEMS_DIR/model" "$STEMS_DIR/cache" "$BUILD_DIR"
echo "Installing into: $STEMS_DIR"

# 1. ONNX Runtime dylib (with CoreML EP) ------------------------------------
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) ORT_PKG="onnxruntime-osx-arm64-$ORT_VER" ; DYLIB="libonnxruntime.dylib" ;;
  Darwin-x86_64) ORT_PKG="onnxruntime-osx-x86_64-$ORT_VER" ; DYLIB="libonnxruntime.dylib" ;;
  Linux-x86_64) ORT_PKG="onnxruntime-linux-x64-$ORT_VER" ; DYLIB="libonnxruntime.so" ;;
  *) echo "error: unsupported platform for the prebuilt ONNX Runtime; install it manually" >&2; exit 1 ;;
esac

if [ ! -f "$STEMS_DIR/$DYLIB" ]; then
  URL="https://github.com/microsoft/onnxruntime/releases/download/v$ORT_VER/$ORT_PKG.tgz"
  echo "Fetching ONNX Runtime $ORT_VER..."
  curl -fL "$URL" -o "$BUILD_DIR/$ORT_PKG.tgz"
  tar -xzf "$BUILD_DIR/$ORT_PKG.tgz" -C "$BUILD_DIR"
  # Pick the real shared library — exclude the dSYM debug copy, the .pc, the
  # bare symlink (-type f), so we don't grab a tiny non-Mach-O file.
  SRC="$(find "$BUILD_DIR/$ORT_PKG/lib" -type f -name 'libonnxruntime*' \
            -not -path '*dSYM*' -not -name '*.pc' | head -1)"
  [ -n "$SRC" ] || { echo "error: libonnxruntime not found in the package" >&2; exit 1; }
  cp "$SRC" "$STEMS_DIR/$DYLIB"
  echo "Installed: $STEMS_DIR/$DYLIB"
fi

# 2. htdemucs_ft ONNX models (4 specialists) --------------------------------
SUFFIX=""
[ "${HM_STEMS_FP16:-0}" = "1" ] && SUFFIX="_fp16weights"
for stem in vocals drums bass other; do
  SRC_NAME="htdemucs_ft_${stem}${SUFFIX}.onnx"
  DEST="$STEMS_DIR/model/htdemucs_ft_${stem}.onnx"   # app expects the bare name
  if [ ! -f "$DEST" ]; then
    URL="https://huggingface.co/$HF_REPO/resolve/main/$SRC_NAME"
    echo "Downloading $SRC_NAME ..."
    curl -fL "$URL" -o "$DEST"
  fi
done

echo
echo "Done. Restart HypeMuzik, then open Stems while a local track plays."
echo "It separates automatically (a few seconds on Apple Silicon), then the pads go live."
