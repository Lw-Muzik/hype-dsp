#!/usr/bin/env bash
#
# Push macOS Developer ID signing + notarization secrets to GitHub Actions.
#
# Prerequisites (see docs/macos-signing.md):
#   • A "Developer ID Application" certificate exported as a .p12 (cert + key).
#   • An app-specific password from appleid.apple.com.
#   • `gh` installed and authenticated (`gh auth login`), run from the repo.
#
# Usage:  ./scripts/setup-macos-signing.sh [path/to/DeveloperID.p12]
#
# Secrets are read from prompts (never passed as args / never echoed) and the
# .p12 is base64-encoded before upload. Nothing is written to disk.
set -euo pipefail

command -v gh >/dev/null || { echo "✖ GitHub CLI (gh) not found — install it first." >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "✖ Not logged in — run 'gh auth login'." >&2; exit 1; }

P12_PATH="${1:-}"
[ -z "$P12_PATH" ] && read -r -p "Path to Developer ID Application .p12: " P12_PATH
[ -f "$P12_PATH" ] || { echo "✖ File not found: $P12_PATH" >&2; exit 1; }

read -r -p 'Signing identity (e.g. "Developer ID Application: Name (TEAMID)"): ' IDENTITY
read -r -p "Apple ID email (notarization): " APPLE_ID
read -r -p "Apple Team ID (e.g. CLQALJFSN4): " TEAM_ID
read -r -s -p ".p12 export password: " P12_PW; echo
read -r -s -p "App-specific password: " APP_PW; echo

[ -n "$IDENTITY" ] && [ -n "$APPLE_ID" ] && [ -n "$TEAM_ID" ] && [ -n "$P12_PW" ] && [ -n "$APP_PW" ] \
  || { echo "✖ All fields are required." >&2; exit 1; }

CERT_B64="$(base64 < "$P12_PATH" | tr -d '\n')"
REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
echo "→ Setting secrets on ${REPO} …"

printf '%s' "$CERT_B64" | gh secret set APPLE_CERTIFICATE
printf '%s' "$P12_PW"   | gh secret set APPLE_CERTIFICATE_PASSWORD
printf '%s' "$IDENTITY" | gh secret set APPLE_SIGNING_IDENTITY
printf '%s' "$APPLE_ID" | gh secret set APPLE_ID
printf '%s' "$APP_PW"   | gh secret set APPLE_PASSWORD
printf '%s' "$TEAM_ID"  | gh secret set APPLE_TEAM_ID

echo "✓ Secrets set. Push a tag to build a signed + notarized DMG:"
echo "    git tag v0.1.0 && git push origin v0.1.0"
