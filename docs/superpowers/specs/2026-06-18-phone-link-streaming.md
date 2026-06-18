# Phone Link — stream the phone's music through the desktop

**Status:** approved design · 2026-06-18
**Repos:** `hypemuzik-desktop` (Tauri 2 / Rust / React) · `hype` (Flutter mobile)

## Goal

A user with music on their phone can play it through the desktop app over the
local network — decoded and run through the desktop's full DSP chain, so it
sounds "as if it were on the laptop." Works **both directions**: browse the
phone from the desktop (*pull*), or tap a song on the phone and cast it to the
desktop (*push*).

## Roles: one data plane, two control planes

- **Data plane (always phone → desktop).** The phone serves the audio file
  bytes; the desktop decodes them (symphonia) and runs them through its DSP
  chain to the speakers. This is the only direction that makes sense — DSP and
  output live on the laptop.
- **Control plane (who *initiates* a play):**
  - **Pull / browse** — desktop initiates. The user browses the phone's library
    on the desktop and hits play. The desktop GETs `/stream/{id}` from the phone.
  - **Push / cast** — phone initiates. The user taps a song on the phone; the
    phone POSTs `/cast {trackId}` to the desktop, which then pulls that same
    `/stream/{trackId}` and plays it. Push is a thin control channel on top of
    the identical data plane.

Pairing establishes **one shared token** used to authenticate calls in both
directions.

## Discovery & pairing

- **mDNS.** Both devices advertise the service type `_hypemuzik._tcp` on the
  LAN. TXT records:
  - `role` = `source` (phone media server) | `player` (desktop control server)
  - `name` = human device name (e.g. "Bruno's iPhone", "MacBook Pro")
  - `id` = stable per-install UUID
  - `v` = protocol version (`1`)
  Each side browses for the *other* role. (Desktop browses `role=source`; phone
  browses `role=player`.)
- **PIN handshake.** On the screen that initiates pairing, one device shows a
  random **6-digit PIN** valid for ~2 minutes. The peer calls `POST /pair`
  with the PIN. On match, the server returns a long-lived random **token**
  (≥32 bytes, base64url). Both sides persist the pairing:
  - Desktop → `app_data_dir/paired-devices.json` (`[{id,name,host,port,token}]`)
  - Phone → allowed-devices list (`{deviceId: {name, token}}`) in prefs.
  After first pairing, reconnects are silent (token already known); the PIN is
  only needed once per device pair.

## Wire protocol (v1)

All requests after pairing carry `Authorization: Bearer <token>`. LAN-only,
plain HTTP (TLS is a future addition). JSON bodies/responses are
`application/json`.

### Phone — media server (`shelf`)
| Method | Path | Body / Query | Response |
|---|---|---|---|
| `POST` | `/pair` | `{pin, deviceId, deviceName}` | `200 {token, deviceId, deviceName}` while a PIN is active; `403` otherwise |
| `GET`  | `/library` | — | `200 {tracks: [Track]}` |
| `GET`  | `/stream/{id}` | `Range:` (optional) | `200`/`206` audio bytes, `Content-Type` by ext, `Accept-Ranges: bytes` |
| `GET`  | `/art/{id}` | — | `200` image bytes or `404` *(M3)* |
| `GET`  | `/ping` | — | `200 {name, id, v}` (liveness; unauthenticated) |

`Track`:
```jsonc
{ "id": "726354", "title": "…", "artist": "…|null", "album": "…|null",
  "durationMs": 215000, "ext": "mp3", "hasArt": true }
```
`id` is the on-device song id (`SongModel.id`); the server maps it to
`SongModel.data` (the real filesystem path) to read bytes.

### Desktop — control server (`tiny_http`) *(M2)*
| Method | Path | Body | Effect |
|---|---|---|---|
| `POST` | `/pair` | `{pin, deviceId, deviceName}` | symmetric pairing (desktop can show the PIN too) |
| `POST` | `/cast` | `{sourceId, trackId}` | desktop pulls `/stream/{trackId}` from the named paired source and plays it |
| `POST` | `/transport` | `{action: "play"\|"pause"\|"next"\|"prev"\|"stop"}` | forwards to the engine |
| `GET`  | `/now` | — | `{playing, title, artist, positionMs, durationMs}` |
| `GET`  | `/ping` | — | `{name, id, v}` |

## Desktop components (Tauri / Rust)

- **`src-tauri/src/link.rs`** — the link module:
  - mDNS browse + advertise via `mdns-sd`.
  - Pairing client (`POST /pair`), token store (`paired-devices.json`,
    load/save mirroring `cloud.rs`'s token store).
  - Library fetch (`GET /library` with Bearer → `Vec<PhoneTrack>`).
  - Resolve `(deviceId, trackId)` → `(stream_url, headers)` and play via the
    **existing** `engine.play_stream(url, headers)` (no engine changes for pull).
  - *(M2)* control server (`tiny_http`) for `/cast`, `/transport`, `/now`, and
    `role=player` advertisement.
- **`src-tauri/src/commands/link.rs`** — `link_discover`, `link_pair`,
  `link_paired`, `link_unpair`, `link_library`, `link_play`. Registered in
  `lib.rs`; `LinkState` managed like `CloudState`.
- **Frontend** — `src/lib/types.ts` (`PhoneDevice`, `PhoneTrack`),
  `src/lib/ipc.ts` wrappers, store wiring (`playPhone`), and a **Devices**
  feature view (`src/features/devices/DevicesView.tsx`) mirroring `CloudView`:
  discovered + paired phones → PIN entry → browse library → play. New route +
  sidebar entry.

## Mobile components (Flutter)

- **`lib/controllers/StreamServerController.dart`** (ChangeNotifier) — start/stop
  the `shelf` server, generate/expire the PIN, manage the token / allowed-devices
  store, advertise via `bonsoir`. Reads the library from `AppController` /
  `on_audio_query`.
- **Server handlers** — `/pair`, `/library`, `/stream/{id}` (with HTTP range),
  `/ping`. Map `id → SongModel.data` to stream bytes.
- **`lib/pages/stream_server.dart`** — a "Stream / Cast" screen reachable from
  `Settings.dart`: server on/off toggle, the pairing PIN, list of paired
  desktops. *(M2)* Cast mode: discover desktops, tap a song → `POST /cast`,
  forward transport.
- **Deps:** `shelf`, `shelf_router`, `bonsoir`, `network_info_plus`.
- **Permissions:** Android `FOREGROUND_SERVICE` + a foreground service so the
  server survives backgrounding (INTERNET / media-read already present);
  iOS `NSLocalNetworkUsageDescription` + `NSBonjourServices` (`_hypemuzik._tcp`).

## Build sequence

1. **M1 — Pull.** Phone media server + mDNS advertise + PIN pairing; desktop
   discover / pair / browse / **play**. Complete, usable feature on its own.
2. **M2 — Cast.** Desktop control server + advertise; phone cast UI (tap →
   plays on the laptop + transport forwarding + now-playing).
3. **M3 — Polish.** Artwork endpoint + desktop covers, two-way now-playing sync,
   silent reconnect, foreground-service hardening, and HTTP range/seek on the
   desktop stream source (enables scrubbing).

## v1 limitations (accepted)

- No scrubbing on streamed tracks (desktop stream source is live/non-seekable;
  range-seek lands in M3).
- Plain HTTP over LAN with token auth; no TLS yet.
- Same Wi‑Fi / subnet only.
- Gradient covers on the desktop until the `/art` endpoint lands (M3).

## Security notes

- Tokens are random ≥256-bit secrets, never logged; transport is LAN-only.
- `POST /pair` only issues a token while the user has an active PIN on screen,
  rate-limited; the PIN expires (~2 min).
- A paired device can be revoked from either side (unpair → token invalidated).
- `/ping` is the only unauthenticated endpoint and returns no library data.
