# Remote Phone Link over iroh (P2P, cross-network)

Status: in progress (2026-06-20)
Owner: desktop + mobile

## Goal

Let a desktop link to a phone's music library **across different networks** (not
just the same LAN), and link **multiple phones** to one desktop with music
fetched from all of them. Keep the existing LAN path as the fast default.

Transport choice: **iroh 1.0** (QUIC with NAT hole-punching + relay fallback).
Media flows peer-to-peer when hole-punching succeeds, relayed only as a fallback.
End-to-end encrypted by construction (connections are keyed to node identities).

## Key idea: transparent TCP-over-QUIC tunnel

The phone already runs an HTTP **shelf** media server (`/library`, `/stream` with
Range, `/art`, `/lyrics`, `/pair`, `/ping`). Instead of re-implementing any of
that over iroh, we tunnel raw TCP bytes over an iroh bi-directional stream:

```
reqwest (desktop)
   → 127.0.0.1:<proxyPort>   (loopback TCP listener, one per remote phone)
      → iroh bi-stream  ───────────────[ QUIC, hole-punched/relayed ]──────────────►
                                                              → 127.0.0.1:<shelfPort> (phone)
                                                                 → existing shelf HTTP server
```

Each inbound TCP connection on the desktop opens one iroh bi-stream; both sides
`copy` bytes in each direction (`tokio::io::copy`). Because QUIC streams are
reliable+ordered, HTTP/1.1 (including Range and keep-alive) works transparently.

Consequence: a remote phone is represented in `LinkState` as a **paired device
with `host = 127.0.0.1`, `port = <its loopback proxy port>`**. Every existing
`hm-link` call (`library`, `stream_target`, `artwork`, `lyrics`) works unchanged.

ALPN: `hypemuzik/link/0`. A second ALPN `hypemuzik/pair/0` for the pairing
control stream (below).

## Pairing across networks (phone scans desktop QR — no desktop camera needed)

Node identities are iroh `EndpointId`s (ed25519 pubkeys, base32). Each app
persists its `SecretKey` so its id is stable.

1. Desktop "Add phone (remote)" shows a **QR** encoding
   `hypemuzik://pair?ep=<desktopEndpointId>&pin=<6-digit>` plus the 6-digit PIN
   in text. A pairing session (pin + expiry) is opened on the desktop.
2. Phone scans the QR (phones have cameras) → parses `ep` + `pin`.
3. Phone **dials the desktop** (`hypemuzik/pair/0`) and sends
   `{ endpointId, name, pin }`.
4. Desktop verifies the pin against its open session, mints a long-lived token,
   replies `{ token, desktopName }`, and stores the phone
   (`Paired { id: phoneEndpointId, name, token, ... }`). The phone stores the
   desktop's endpointId + token as a trusted peer.
5. Thereafter the **desktop dials the phone** (`hypemuzik/link/0`) for the media
   tunnel; iroh discovery resolves the phone's endpointId via relay/DNS so it
   works on any network. The token authenticates each tunnel (sent once per
   connection, validated by the phone before proxying).

Silent reconnect: desktop redials stored endpointIds on startup / when the user
opens the remote phone; no PIN needed (token in the connection preamble).

## Multiple phones

`LinkState` already stores a `Vec<Paired>` and `useMusicLibrary` already
aggregates every paired phone's library (skipping offline ones). For remote
phones we keep one iroh connection + one loopback proxy listener **per phone**,
so several remote phones coexist with several LAN phones. UI: a managed list of
linked phones (LAN + remote) with per-phone online/offline + unpair.

## Crates / files

- **`crates/hm-remote`** (new, shared Rust): the iroh endpoint, the tunnel
  (both the desktop *client* side and the phone *server/accept* side), the
  pairing control protocol, and a stable `SecretKey` store. Desktop links it
  directly; the phone links it via `flutter_rust_bridge` (M2).
  - `Endpoint` builder, persisted key, relay = default (n0) for now (self-host
    later).
  - `serve_tunnel(shelf_port, token_check)` — accept loop, bi-stream → localhost
    shelf proxy (phone side).
  - `RemoteClient::open_proxy(endpoint_id) -> proxy_port` — desktop side: dial +
    loopback TCP listener that tunnels to the phone.
  - pairing: `accept_pairing(session)` (desktop) / `dial_pair(...)` (phone).
- **`src-tauri/src/commands/link.rs`** — new commands: `link_remote_qr` (open a
  pairing session, return QR payload + pin), `link_remote_status`, reuse
  `link_paired`/`link_unpair`/`link_library`. Remote phones funnel into the same
  `LinkState` so the rest is unchanged.
- **Desktop UI** — `DevicesView`: "Add phone" → LAN list + "Across networks (QR)"
  panel showing the QR; a unified "Linked phones" list.
- **Mobile (`hype`)** — `flutter_rust_bridge` binding to `hm-remote`; a QR
  scanner (`mobile_scanner`) on the Stream/Cast screen → `dial_pair`; keep the
  iroh endpoint alive alongside the shelf server (reuse the existing foreground
  service).

## Phases

- **M1 (this session): desktop transport foundation.** `hm-remote` crate: iroh
  endpoint + TCP-over-QUIC tunnel both directions, with an in-process two-endpoint
  integration test proving an HTTP request round-trips over iroh to a dummy shelf.
  No UI yet. Compiles + tests pass.
- **M2: pairing + LinkState integration (desktop).** Pairing control protocol,
  QR payload, token store, remote phones surface as loopback-proxy paired
  devices; commands + minimal UI. Desktop-testable against a mock phone endpoint.
- **M3: phone side.** `flutter_rust_bridge` to `hm-remote`, QR scanner, keep
  endpoint alive in the foreground service. **Needs device build + testing.**
- **M4: multi-phone UI polish + self-hosted relay option.**

## Risks / notes

- iroh pulls a large dep tree (tokio, quinn) — first build is slow; isolated in
  `hm-remote`.
- Phone NDK cross-compile (flutter_rust_bridge + cargokit) is the main unknown;
  cannot be verified without a device. Flagged in M3.
- Default relay is n0's public servers; a self-hosted `iroh-relay` is a later
  privacy/control upgrade (M4).
- Token is sent in the connection preamble over an already-encrypted QUIC stream.
