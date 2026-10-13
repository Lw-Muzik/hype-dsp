#!/usr/bin/env bash
#
# Developer shortcut for what the app does itself on first use of Stems:
#   1. ONNX Runtime 1.22 library          -> <stems>/runtime/libonnxruntime.dylib
#   2. the four Spleeter nets (fp16 ONNX) -> <stems>/models/spleeter-4stems-fp16-v1/*.onnx
#
# Deezer's Spleeter (code and pretrained models MIT) as exported to ONNX by
# Best-Practice/spleeter-4stems-onnx; the same files the mobile app downloads,
# served from hypemuzik.com. Every download is checked against the SHA-256 pinned
# below (and in the app) and refused on a mismatch.
#
# Requires: curl, tar, shasum (or sha256sum). ~80 MB of models + ~26 MB runtime.
set -euo pipefail

# ORT >= 1.22 is required: `ort` 2.0.0-rc.10 asks for API version 22.
ORT_VER="1.22.0"
MODEL_DIR_NAME="spleeter-4stems-fp16-v1"
MODEL_BASE="${HM_STEMS_MODEL_BASE:-https://hypemuzik.com/models/stems/$MODEL_DIR_NAME}"

# name sha256 (keep in sync with crates/hm-stems/src/download.rs and
# hypemuzik-site/deploy/models.sha256)
MODELS="
vocals db47148ab1c52709ce694893f532c91abfe3edc4d46238939570e036a22878ca
drums 7ae4002e5633634674f74dc3356d5875b0da894d59ce0f60e844bb8f9cb8aa92
bass ba4c4949a27222492cca49859901a873b4b71461dc48c7c5a51f93d31eb11f55
other 3cc59116cb7195946ab9596d8ca25984d09c0f8a70db8cf85d063132f97bc61d
"

APP_ID="${HM_BUNDLE_ID:-com.hypemuzik.desktop}"
case "$(uname -s)" in
  Darwin) DEFAULT_DIR="$HOME/Library/Application Support/$APP_ID/stems" ;;
  Linux)  DEFAULT_DIR="$HOME/.local/share/$APP_ID/stems" ;;
  *)      DEFAULT_DIR="$HOME/.hypemuzik/stems" ;;
esac
STEMS_DIR="${HM_STEMS_DIR:-$DEFAULT_DIR}"
BUILD_DIR="${HM_STEMS_BUILD:-$HOME/.cache/hm-stems-build}"
mkdir -p "$STEMS_DIR/models/$MODEL_DIR_NAME" "$STEMS_DIR/runtime" "$BUILD_DIR"
echo "Installing into: $STEMS_DIR"

sha256_of() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  else sha256sum "$1" | cut -d' ' -f1; fi
}

# fetch URL DEST SHA256 -- download to DEST.part, verify, then move into place.
fetch() {
  local url="$1" dest="$2" want="$3"
  curl -fL "$url" -o "$dest.part"
  local got
  got="$(sha256_of "$dest.part")"
  if [ "$got" != "$want" ]; then
    rm -f "$dest.part"
    echo "error: $(basename "$dest") failed its integrity check (got $got)" >&2
    exit 1
  fi
  mv "$dest.part" "$dest"
}

# 1. ONNX Runtime library --------------------------------------------------
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)  ORT_PKG="onnxruntime-osx-arm64-$ORT_VER";  ORT_SHA="cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07"; MEMBER="lib/libonnxruntime.$ORT_VER.dylib"; LIB="libonnxruntime.dylib" ;;
  Darwin-x86_64) ORT_PKG="onnxruntime-osx-x86_64-$ORT_VER"; ORT_SHA="e4ec94a7696de74fb1b12846569aa94e499958af6ffa186022cfde16c9d617f0"; MEMBER="lib/libonnxruntime.$ORT_VER.dylib"; LIB="libonnxruntime.dylib" ;;
  Linux-x86_64)  ORT_PKG="onnxruntime-linux-x64-$ORT_VER";  ORT_SHA="8344d55f93d5bc5021ce342db50f62079daf39aaafb5d311a451846228be49b3"; MEMBER="lib/libonnxruntime.so.$ORT_VER";    LIB="libonnxruntime.so" ;;
  *) echo "error: no prebuilt ONNX Runtime pinned for this platform; install it and set ORT_DYLIB_PATH" >&2; exit 1 ;;
esac

if [ ! -f "$STEMS_DIR/runtime/$LIB" ]; then
  echo "Fetching ONNX Runtime $ORT_VER ..."
  fetch "https://github.com/microsoft/onnxruntime/releases/download/v$ORT_VER/$ORT_PKG.tgz" "$BUILD_DIR/$ORT_PKG.tgz" "$ORT_SHA"
  tar -xzf "$BUILD_DIR/$ORT_PKG.tgz" -C "$BUILD_DIR" "$ORT_PKG/$MEMBER"
  cp "$BUILD_DIR/$ORT_PKG/$MEMBER" "$STEMS_DIR/runtime/$LIB"
  echo "Installed: $STEMS_DIR/runtime/$LIB"
fi

# 2. Spleeter nets -----------------------------------------------------------
echo "$MODELS" | while read -r stem sha; do
  [ -n "$stem" ] || continue
  DEST="$STEMS_DIR/models/$MODEL_DIR_NAME/$stem.onnx"
  if [ -f "$DEST" ] && [ "$(sha256_of "$DEST")" = "$sha" ]; then continue; fi
  echo "Downloading $stem.onnx ..."
  fetch "$MODEL_BASE/$stem.onnx" "$DEST" "$sha"
done

echo
echo "Done. Restart HypeMuzik, then open Stems while a track plays."
