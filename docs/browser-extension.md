# Browser extension bridge — 5.1 streaming (design only)

> **Status: design / not built this pass.** This documents the production design
> for handling DRM-protected streaming surround (e.g. Netflix 5.1). No extension
> is implemented here.

## Problem

DRM-protected streaming (Widevine/PlayReady/FairPlay) decrypts and renders audio
inside a protected path the desktop app cannot tap — a system virtual device
either receives a downmixed stereo signal or nothing usable for surround. To
apply HypeMuzik's spatializer to a 5.1 streaming title, the processing must
cooperate with the page that owns the decoded audio.

## Production design

A **browser extension** cooperates with the desktop app over a **local bridge**:

```
[ streaming page ]
      │  Web Audio API graph (per-tab)
      ▼
[ browser extension content/worker ]  ──local WebSocket / native messaging──►  [ HypeMuzik desktop ]
      ▲                                                                              │
      └──────────────── enhancement params (EQ / surround / gain) ◄─────────────────┘
```

- The extension attaches a Web Audio graph to the tab's media element and applies
  enhancement **in the page**, where the decoded audio is reachable, mirroring
  the desktop chain's parameters.
- A **native-messaging host** or a **local WebSocket** (loopback only,
  origin-checked, token-handshaked) carries parameters from the desktop app to
  the extension and telemetry back.
- The desktop app remains the source of truth for presets/profiles; the
  extension is a thin remote DSP applying the same parameter model.

## Optional inert local endpoint

A future iteration may scaffold an inert local bridge endpoint in `src-tauri`
that accepts a connection and replies "not implemented", so the wiring exists
without behavior. It is intentionally absent today.

## Security notes

- Bind the bridge to loopback only; never expose it on the network.
- Require a per-session token negotiated through the extension's options page.
- Validate message origin and shape; treat the page as untrusted.
