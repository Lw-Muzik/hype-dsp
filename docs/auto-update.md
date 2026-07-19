# Auto-update

HypeMuzik updates itself. The app checks on a cadence, downloads a new version
in the background, and installs it the next time the user quits — the one moment
nothing is playing and the audio tap is already torn down.

This document is the operator's guide: the one-time setup, how a release flows,
and the failure modes worth knowing before they happen in production.

## How it works, in one paragraph

Every build is signed with a minisign key. On launch (after 90 s) and every 6 h,
the app fetches `latest.json` from GitHub Releases — falling back to CrabNebula
if GitHub is unreachable — compares versions, and if a newer one exists downloads
the bundle for its platform and verifies the signature against the public key
compiled into the app. A verified update is held in memory until quit, then
written. The whole flow lives in `src-tauri/src/updater.rs`; the UI is one row in
Settings → About.

## One-time setup

### 1. The signing key (already generated)

A keypair was generated into `~/.tauri/` (outside the repo):

- `hypemuzik-updater.key` — **private. Never commit. Back this up somewhere you
  will still have in a year.** If it is lost, see *Key rotation* below — there is
  no recovery, only a migration path, and it only works while you still have the
  old key.
- `hypemuzik-updater.key.pub` — public. Already committed, as `plugins.updater.pubkey`
  in `src-tauri/tauri.conf.json`.
- `hypemuzik-updater.password` — the private key's passphrase.

### 2. GitHub secrets

The release workflow signs with two secrets. Set them once:

```sh
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/hypemuzik-updater.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD < ~/.tauri/hypemuzik-updater.password
```

Without these the build still succeeds — the bundler only *warns* on a missing
key — but the updater artifacts are unsigned and rejected on every user's
machine. The `Verify updater artifacts are signed` step fails the release rather
than let that ship.

### 3. CrabNebula (fallback host, optional but recommended)

GitHub Releases is the primary and the durable archive. CrabNebula is the CDN
fallback and gives download analytics. To enable it, set:

```sh
gh secret set CN_API_KEY        # from CrabNebula Cloud → org settings
gh secret set CN_APP_SLUG       # e.g. "your-org/hypemuzik"
```

If these are absent the CrabNebula publish step is skipped and updates still work
entirely through GitHub. **CrabNebula auto-deletes releases with no downloads
after 90 days** — this is why GitHub, not CrabNebula, is the archive of record.

## Cutting a release

```sh
# bump version in src-tauri/tauri.conf.json first, then:
git tag v0.2.0 && git push origin v0.2.0
```

The workflow builds all three platforms, signs them, publishes a **non-draft**
GitHub Release (the updater cannot see a draft — a draft ships no updates and
looks like it is working), verifies the manifest, and optionally mirrors to
CrabNebula.

> The old flow published *drafts* for manual review. That is incompatible with
> auto-update: a human gate before publish means a human gate before anyone can
> update. If you want review, do it on a release-candidate tag (`v0.2.0-rc.1`)
> that users are not on, then tag the real version.

## What the CI guards, and why each exists

Both of these protect against **silent** failures — the build stays green while
the update is dead on user machines:

- **Unsigned artifacts** → the bundler warns, never errors. `Verify updater
  artifacts are signed` counts `.sig` files and fails if there are none.
- **A missing platform** → `latest.json` builds fine without, say, the
  `darwin-x86_64` key, and Intel Mac users then see "no update" forever. A
  universal macOS build needs **both** `darwin-aarch64` and `darwin-x86_64` —
  there is no `darwin-universal`. `verify-manifest` asserts all four required
  keys are present and signed.

## Key rotation

There is one `pubkey` baked into every shipped binary, so a key can only be
changed *through* an update signed by the current key:

1. Generate a new keypair.
2. Put the new **public** key in `tauri.conf.json`.
3. Release that version **signed with the OLD private key**. Users on the old key
   accept it (old signature), and it carries the new public key forward.
4. From the next release on, sign with the new private key.

This works only while you still hold the old key. Lose it and there is no path
that reaches already-installed apps — they can only be updated by hand.

## Per-platform reality

- **macOS**: works with the notarized `.app`; the staple ticket survives the
  `.tar.gz` the updater consumes. Universal build ⇒ both arch keys.
- **Windows**: `installMode: passive` shows a progress UI with no prompts. The
  installer ends by exiting the process outright — no destructors run — which is
  exactly why teardown is explicit in `updater.rs` rather than left to `Drop`.
- **Linux**: **AppImage self-updates.** `.deb`/`.rpm` are technically supported
  but each install triggers a `pkexec`/`sudo` password prompt, so they are left
  to the package manager — those users get the app through `apt`/`dnf`, not
  through this. `.snap` updates via snapd on its own.

## Known limits

- The download is buffered **entirely in RAM** with no resume. A ~45 MB bundle is
  held in memory while staged, and an interrupted download restarts from zero on
  the next cadence tick.
- Endpoint failover covers connection errors and non-2xx responses, but **not** a
  `200` carrying an unparseable body (a CDN error page under a 200). That aborts
  the check with no fallback. Unfixable from our side; worth knowing.
- No staged/percentage rollout and no server-side kill switch. To pull a bad
  release: delete its GitHub Release (users stop being offered it) and cut a
  higher version. There is no way to *downgrade* an app that already updated —
  the default version comparator only moves forward.
- Up to 2 s of the very latest settings changes can be lost on an
  install-at-quit, because the settings autosave runs on a 2 s debounce and there
  is no flush hook. Everything older is already on disk.
