# Cloud music setup (Google Drive & Dropbox)

HypeMuzik streams audio from Google Drive and Dropbox. Desktop OAuth uses a
**loopback redirect + PKCE**, so you must register a desktop OAuth client for each
provider and point HypeMuzik at the credentials via environment variables.

The fixed redirect URI is **`http://localhost:53682`** — register exactly that.

## Credentials → environment variables

| Variable | Provider |
|---|---|
| `HM_GDRIVE_CLIENT_ID` | Google Drive OAuth **Desktop app** client id |
| `HM_GDRIVE_CLIENT_SECRET` | Google Drive client secret (installed-app, not confidential) |
| `HM_DROPBOX_APP_KEY` | Dropbox app key |

In development, launch with them set:

```bash
HM_GDRIVE_CLIENT_ID=… HM_GDRIVE_CLIENT_SECRET=… HM_DROPBOX_APP_KEY=… pnpm tauri dev
```

The **Cloud** page shows "not configured" for any provider whose variable is empty.

## Google Drive
1. <https://console.cloud.google.com> → create/select a project.
2. **APIs & Services → Library** → enable the **Google Drive API**.
3. **OAuth consent screen** → add the scope
   `https://www.googleapis.com/auth/drive.readonly` (and add yourself as a test
   user while the app is unverified).
4. **Credentials → Create credentials → OAuth client ID → Application type:
   Desktop app.** Copy the **client ID** and **client secret**.
   - Google desktop clients accept loopback redirects automatically; no need to
     register `http://localhost:53682` explicitly.
5. Set `HM_GDRIVE_CLIENT_ID` / `HM_GDRIVE_CLIENT_SECRET`.

## Dropbox
1. <https://www.dropbox.com/developers/apps> → **Create app** →
   **Scoped access** → **Full Dropbox** (or App folder).
2. **Permissions** tab → enable `files.metadata.read` and `files.content.read`
   → **Submit**.
3. **Settings** tab → under **OAuth 2 → Redirect URIs**, add
   **`http://localhost:53682`**.
4. Copy the **App key** → set `HM_DROPBOX_APP_KEY`.

## How it works
- Connecting opens your browser to the provider's consent page; after you approve,
  the redirect lands on HypeMuzik's local listener (`localhost:53682`), the code
  is exchanged for tokens (PKCE), and tokens are saved (and refreshed
  automatically) in the app-data dir.
- Playback streams the file through the full enhancement chain: Dropbox via a
  temporary direct link; Google Drive via `files/{id}?alt=media` with a bearer
  token.
