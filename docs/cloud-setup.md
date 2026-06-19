# Cloud music setup (Google Drive & Dropbox)

HypeMuzik streams audio from Google Drive and Dropbox. Desktop OAuth uses a
**loopback redirect + PKCE** (fixed redirect URI **`http://localhost:53682`**).

## Bundled defaults (reused from the Hype mobile app)

The desktop ships with the **mobile app's** OAuth identifiers baked in, so you
don't need new credentials:

- **Dropbox app key** — works as-is, with **one one-time console step**: add
  `http://localhost:53682` to this app's **Redirect URIs** (the desktop's
  loopback flow differs from the phone's custom-scheme redirect). Then Dropbox
  connects with no env vars.
- **Google client ID + secret** — a dedicated **Desktop app** OAuth client is
  baked in, so Google Drive connects with no env vars. (Desktop-client secrets
  are non-confidential by Google's design.) Access is limited to the **test
  users** on the consent screen until the app is verified, so add your Google
  account there.

Environment variables still override the bundled defaults:

| Variable | Provider | Needed? |
|---|---|---|
| `HM_DROPBOX_APP_KEY` | Dropbox app key | optional (bundled) |
| `HM_GDRIVE_CLIENT_ID` | Google OAuth client id | optional (bundled) |
| `HM_GDRIVE_CLIENT_SECRET` | Google client secret | optional (bundled) |

The **Cloud** page shows "not configured" only if you blank out the bundled
identifiers.

## Google Drive (bundled — steps below are for using your own project)

A **Desktop app** client is already bundled. To point at **your own** Google
Cloud project instead, create a Desktop-app client and set `HM_GDRIVE_CLIENT_ID`
+ `HM_GDRIVE_CLIENT_SECRET`:

1. <https://console.cloud.google.com> → select that project.
2. **APIs & Services → Library** → ensure the **Google Drive API** is enabled.
3. **OAuth consent screen** → add the scope
   `https://www.googleapis.com/auth/drive.readonly` (add yourself as a test user
   while unverified).
4. **Credentials → Create credentials → OAuth client ID → Application type:
   Desktop app.** Copy the **client ID** and **client secret**.
   - Desktop clients accept loopback redirects automatically.
5. Set `HM_GDRIVE_CLIENT_ID` (the new desktop client) and
   `HM_GDRIVE_CLIENT_SECRET`. (Or keep the bundled web client ID and set only
   the secret, but then also add `http://localhost:53682` to that client's
   authorized redirect URIs.)

## Dropbox (just add the redirect)

The app key is bundled; you only need to allow the desktop's redirect:

1. <https://www.dropbox.com/developers/apps> → open the existing Hype app
   (app key `1d0mou7l0x19mas`).
2. **Settings** tab → under **OAuth 2 → Redirect URIs**, add
   **`http://localhost:53682`**.
3. (If not already enabled) **Permissions** tab → `files.metadata.read` and
   `files.content.read` → **Submit**.

That's it — Dropbox connects with no env vars.

## How it works
- Connecting opens your browser to the provider's consent page; after you approve,
  the redirect lands on HypeMuzik's local listener (`localhost:53682`), the code
  is exchanged for tokens (PKCE), and tokens are saved (and refreshed
  automatically) in the app-data dir.
- Playback streams the file through the full enhancement chain: Dropbox via a
  temporary direct link; Google Drive via `files/{id}?alt=media` with a bearer
  token.
