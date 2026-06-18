# macOS code signing & notarization

HypeMuzik is configured to sign and notarize for **distribution** so the `.dmg`
opens cleanly on any Mac (no "unidentified developer" / Gatekeeper block).

Signing is **credential-driven** — nothing is hardcoded:

- `src-tauri/tauri.conf.json` sets `hardenedRuntime: true` + `entitlements.plist`
  (audio-input + library-validation exceptions) but **no** `signingIdentity`.
- Tauri picks the identity from the **`APPLE_SIGNING_IDENTITY`** env var and
  **auto-notarizes** when the notarization variables are present at build time.

This works both locally (via `.env.signing`) and in CI (via repo secrets, already
wired in `.github/workflows/release.yml`).

> You need an **Apple Developer Program** membership. Team ID for this app is
> `CLQALJFSN4` (Bruno Mugamba). A **"Developer ID Application"** certificate is
> required — the "Apple Development" cert can't be distributed or notarized.

---

## 1. Create a "Developer ID Application" certificate

1. **Keychain Access → Certificate Assistant → Request a Certificate From a
   Certificate Authority.** Enter your Apple ID email, choose **Saved to disk**,
   save `CertificateSigningRequest.certSigningRequest`.
2. Go to <https://developer.apple.com/account/resources/certificates/list> →
   **+** → **Developer ID Application** → upload the CSR → **Download** the
   `.cer` → double-click it to install into your **login** keychain.
3. Verify it's there:
   ```bash
   security find-identity -v -p codesigning | grep "Developer ID Application"
   ```

## 2. Export the certificate as a `.p12`

In **Keychain Access → My Certificates**, expand the new
`Developer ID Application: … (CLQALJFSN4)`, select **both** the certificate and
its private key → right-click → **Export 2 items…** → save as
`DeveloperID.p12` and set a strong password.

> That password is **`APPLE_CERTIFICATE_PASSWORD`**. The exact row name
> (`Developer ID Application: … (CLQALJFSN4)`) is **`APPLE_SIGNING_IDENTITY`**.

## 3. Create an app-specific password (for notarization)

<https://appleid.apple.com> → **Sign-In & Security → App-Specific Passwords** →
generate one. That value is **`APPLE_PASSWORD`** (your Apple ID email is
`APPLE_ID`).

## 4. CI: push the secrets to GitHub

From the repo root, run (uploads all six secrets via `gh`, prompting for the
passwords so they never hit your shell history):

```bash
./scripts/setup-macos-signing.sh path/to/DeveloperID.p12
```

Sets: `APPLE_CERTIFICATE` (base64 of the .p12), `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`.

Then trigger a build:

```bash
git tag v0.1.0 && git push origin v0.1.0   # → draft release with a signed, notarized DMG
```

## 5. Local signed build (optional)

Create `.env.signing` in the repo root (it's gitignored) and run
`./scripts/build-macos-signed.sh`:

```dotenv
# .env.signing  — DO NOT COMMIT
APPLE_SIGNING_IDENTITY="Developer ID Application: Bruno Mugamba (CLQALJFSN4)"
APPLE_ID="you@example.com"
APPLE_PASSWORD="abcd-efgh-ijkl-mnop"   # app-specific password
APPLE_TEAM_ID="CLQALJFSN4"
```

> The cert + key must be in your login keychain (steps 1–2). For local builds the
> `.p12`/base64 isn't needed — Tauri signs straight from the keychain.

## Verifying a build

```bash
APP="target/universal-apple-darwin/release/bundle/macos/HypeMuzik.app"
codesign -dvvv --verbose=4 "$APP"      # signed by Developer ID, hardened runtime
spctl -a -vvv -t install "$APP"        # → "accepted … source=Notarized Developer ID"
xcrun stapler validate "$APP"          # notarization ticket stapled
```

## Notes

- Alternative to Apple-ID notarization: an **App Store Connect API key**
  (`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`) — set those instead
  of `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` if you prefer.
- `entitlements.plist` keeps the **audio-input** entitlement — required for the
  microphone capture and the system-audio process tap to work under the hardened
  runtime. Don't remove it.
- The system-wide audio tap also needs this signed build to retain its
  audio-capture permission across launches.
