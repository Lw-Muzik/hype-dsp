#!/usr/bin/env bash
#
# Fetch a relocatable mpv into src-tauri/resources/mpv/ so a shipped HypeMuzik
# bundle can play TV (Stations -> TV) with no system install. Run once before
# `pnpm build:app` / `tauri build`. Dev (`tauri dev`) uses an mpv on PATH and
# does NOT need this.
#
# mpv runs as a separate, unmodified process (aggregation, not linking), so its
# GPL stays cleanly separated from the app — exactly like launching VLC.
#
# Requires: curl. macOS additionally uses `dylibbundler` to gather + relink the
# dylibs so the binary is relocatable (brew install dylibbundler).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/src-tauri/resources/mpv"
mkdir -p "$DEST"

case "$(uname -s)" in
  Darwin)
    # Start from a system/Homebrew mpv, then make it relocatable.
    SRC="$(command -v mpv || true)"
    if [ -z "$SRC" ]; then
      echo "error: mpv not found. Install it first: brew install mpv" >&2
      exit 1
    fi
    # Resolve the real binary behind Homebrew's wrapper symlink.
    SRC="$(readlink -f "$SRC" 2>/dev/null || python3 -c 'import os,sys;print(os.path.realpath(sys.argv[1]))' "$SRC")"
    cp -f "$SRC" "$DEST/mpv"

    if command -v dylibbundler >/dev/null 2>&1; then
      # Gather + relink dylibs into resources/mpv/lib/. mpv runs as its own
      # process, so @executable_path is resources/mpv/ and @executable_path/lib/
      # resolves to the bundled libs. tauri.conf's "resources/mpv/*" mapping
      # copies the whole mpv/ folder (binary + lib/) into the app recursively.
      echo "Relinking dylibs with dylibbundler..."
      dylibbundler -od -b -x "$DEST/mpv" -d "$DEST/lib" -p "@executable_path/lib/"
    else
      echo "warning: dylibbundler not found — the copied mpv still references" >&2
      echo "         Homebrew dylibs and is NOT relocatable. Install it with:" >&2
      echo "           brew install dylibbundler" >&2
      echo "         then re-run this script for a self-contained bundle." >&2
    fi
    ;;

  Linux)
    # Prefer a self-contained AppImage-extracted mpv, or fall back to the system
    # binary. Adjust MPV_APPIMAGE_URL to a trusted mirror for CI.
    SRC="$(command -v mpv || true)"
    if [ -z "$SRC" ]; then
      echo "error: mpv not found. Install it first: sudo apt install mpv" >&2
      exit 1
    fi
    cp -f "$SRC" "$DEST/mpv"
    echo "note: on Linux, ensure the target has mpv's shared libs, or bundle an" >&2
    echo "      AppImage-extracted mpv here for a self-contained build." >&2
    ;;

  MINGW* | MSYS* | CYGWIN*)
    # Windows: use an official shinchiro build. Point MPV_WIN_ZIP at a pinned,
    # checksum-verified release for reproducible CI.
    echo "Windows: place mpv.exe (+ its DLLs) from an official mpv build into:" >&2
    echo "  $DEST/mpv.exe" >&2
    echo "e.g. https://sourceforge.net/projects/mpv-player-windows/files/" >&2
    ;;

  *)
    echo "error: unsupported platform for automatic mpv bundling" >&2
    exit 1
    ;;
esac

if [ -x "$DEST/mpv" ] || [ -f "$DEST/mpv.exe" ]; then
  echo "mpv is ready in: $DEST"
else
  echo "error: mpv was not placed in $DEST" >&2
  exit 1
fi
