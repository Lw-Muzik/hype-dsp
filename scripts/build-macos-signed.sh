#!/usr/bin/env bash
#
# Build a signed + notarized universal macOS DMG locally.
#
# Reads signing/notarization credentials from `.env.signing` (gitignored) in the
# repo root, then runs the universal Tauri build. Tauri signs with
# APPLE_SIGNING_IDENTITY and auto-notarizes when the APPLE_ID/PASSWORD/TEAM_ID
# (or APPLE_API_*) variables are present.
#
# Create `.env.signing` from the template in docs/macos-signing.md, then:
#   ./scripts/build-macos-signed.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$ROOT/.env.signing"

[ -f "$ENV_FILE" ] || {
  echo "✖ Missing $ENV_FILE — see docs/macos-signing.md for the template." >&2
  exit 1
}

# Export every assignment in the env file.
set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

: "${APPLE_SIGNING_IDENTITY:?Set APPLE_SIGNING_IDENTITY in .env.signing}"

# Ensure both architectures are installed for the universal target.
rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1 || true

echo "→ Building signed universal DMG as: $APPLE_SIGNING_IDENTITY"
pnpm tauri build --target universal-apple-darwin

echo "✓ Output: target/universal-apple-darwin/release/bundle/dmg/"
