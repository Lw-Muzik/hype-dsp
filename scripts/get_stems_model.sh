#!/usr/bin/env bash
#
# Install the stem-separation engine for HypeMuzik Desktop:
#   1. ONNX Runtime (the CoreML-enabled Apple build) → libonnxruntime.dylib
#   2. htdemucs exported to ONNX                       → model/htdemucs.onnx (+ .json)
#
# The app loads the dylib at runtime (ORT_DYLIB_PATH) and separates the playing
# track in-process on the Neural Engine. Run once.
#
# Requires: curl, tar, python3 with `pip install "torch>=2.1" demucs onnx`.
set -euo pipefail

# ORT version must match what `ort` 2.0.0-rc.10 targets. Override with HM_ORT_VER.
ORT_VER="${HM_ORT_VER:-1.20.1}"

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
  echo "Fetching ONNX Runtime $ORT_VER…"
  curl -fL "$URL" -o "$BUILD_DIR/$ORT_PKG.tgz"
  tar -xzf "$BUILD_DIR/$ORT_PKG.tgz" -C "$BUILD_DIR"
  # The dylib is versioned (libonnxruntime.1.20.1.dylib) — copy to the bare name.
  SRC="$(find "$BUILD_DIR/$ORT_PKG/lib" -name 'libonnxruntime.*' -type f | head -1)"
  [ -n "$SRC" ] || { echo "error: libonnxruntime not found in the package" >&2; exit 1; }
  cp "$SRC" "$STEMS_DIR/$DYLIB"
  echo "Installed: $STEMS_DIR/$DYLIB"
fi

# 2. htdemucs → ONNX --------------------------------------------------------
if [ ! -f "$STEMS_DIR/model/htdemucs.onnx" ]; then
  echo "Exporting htdemucs to ONNX (first run downloads the ~80 MB weights)…"
  python3 "$(dirname "$0")/export_demucs_onnx.py" --out "$STEMS_DIR/model/htdemucs.onnx"
fi

echo
echo "Done. Restart HypeMuzik → open Stems while a local track plays."
echo "It separates automatically (a few seconds on Apple Silicon), then the faders go live."
