# Shell restructure: update notice, Settings window, Equalizer mode

Date: 2026-10-13 · Branch: `feat/shell-settings-window-eq-mode`

## What the user asked for

1. When an update exists, tell the user somewhere they can't miss while using the app.
2. Take Settings out of the sidebar. A button opens it in a **child window that only
   has a close button**. Settings changed there must take effect in the main window
   immediately, and nothing Settings does today may change behaviour ("then sync them,
   make sure the functionality is not altered").
3. Switching to Equalizer mode must not require a trip to Settings.
4. In Equalizer mode show only the audio-tuning views — **Stems, Enhancer, Equalizer,
   Visualizers** — and restructure that UI so every feature is easy to reach.

"Equalizer mode" is the existing system-wide EQ (`useSystemEqStore`, Settings →
System-wide audio → Enable): the app equalizes all system audio instead of playing
its own library. There is no separate "mode" concept in the code today.

## 1. Update notice

Today the updater (Rust, `src-tauri/src/updater.rs`) checks 90 s after launch and
every 6 h, downloads quietly, and installs at quit. Its status reaches the UI only
as one row in Settings → About, which nobody sees.

- `Status::Downloading` gains `version: String` so the UI can say what is coming.
- New `src/features/update/` in the main window:
  - `updateNotice.ts` — pure `noticeFor(status, dismissedVersion) -> Notice | null`
    (`downloading` → a pill with percent or indeterminate; `ready` → the banner unless
    dismissed for that version, else the pill; idle/checking/failed → nothing).
  - `UpdateBanner` — full-width accent banner directly under the top bar, in both
    modes: "HypeMuzik 0.1.30 is ready — restart to update", **Restart now**,
    **What's new** (expands the release notes), **Later**.
  - `UpdatePill` — in the top bar. "Later" only collapses the banner into this pill
    for that version; the notice never disappears until the update is installed.
    Clicking the pill brings the banner back.
- The store subscribes to `updater://status` and calls `updater_status` on mount, so
  a window opened late still shows the current state. A failed check stays a
  Settings-only detail: it isn't the user's problem.

## 2. Settings in a child window

### Window

- Rust command `settings_open(section: Option<String>)` (also bound to a
  **Settings… ⌘,** item in the macOS app menu): focuses the existing `settings`
  window, or builds one: `index.html#<section>`, title "Settings", 760×820, min
  560×480, resizable, `minimizable(false)`, `maximizable(false)`, parent = `main`
  (so it stays above the main window and moves with it), dark theme, same background
  as `main`.
- **Only a close button.** Windows/Linux: disabling minimize and maximize leaves
  the native ✕ alone. macOS: Overlay title bar, and the miniaturize and zoom traffic
  lights are hidden through `objc2-app-kit`, leaving the red close.
- The macOS "close hides the app" handler is scoped to the `main` label, so closing
  Settings really closes it. When `main` hides or closes, Settings closes too.
- Capabilities: the `default` capability lists `settings` next to `main` (it needs
  `dialog` for the library folder picker, `opener` for links, `core` events).
- Asking for an open window at another section emits `settings://section` to it,
  and it scrolls there.

### Frontend

- `src/main.tsx` branches on the current window's label. `settings` renders
  `SettingsWindow` (`src/features/settings/window/`): a draggable header with the
  title (macOS inset for the traffic light), the existing `SettingsView` body,
  `Toaster`, and `SupporterUpsellModal`. It runs **none** of the main window's
  global effects (Hype Connect runtime, media keys, open-with files, onboarding,
  session bootstrap, phone discovery, system-EQ resume).
- `SettingsView` sections get stable ids (`about`, `appearance`, `library`,
  `devices`, `cloud`, `youtube-music`, `playback`, `output`, `system-audio`,
  `visualizer`) for deep links.
- `settings` leaves the `Route` union, the sidebar and the router. A gear button in
  the top bar (both modes) and ⌘, open the window. Callers that used
  `setRoute("settings")` open the matching section instead: Explore →
  `youtube-music`, Library cloud connect → `cloud`.

### Keeping the two windows in sync

(See the sync inventory below.) A small `src/lib/windowSync.ts` keeps chosen keys
of a Zustand store identical in every window:

- `syncStore(store, name, { keys, merge? })`. When one of `keys` changes locally,
  only the changed keys are emitted on `hm://sync` with the sender's label. Other
  windows apply them without re-emitting (an echo guard). `merge` names object keys
  (the engine's `state`) that are diffed and merged field by field, so a window can
  never overwrite fields another window changed meanwhile.
- A window that starts up emits `hm://sync-request`. Every other window answers
  with a full snapshot of its synced keys, so Settings opens showing exactly what
  the main window has, not a stale `localStorage` read.
- Persistence stays where it is today (each action writes its own `localStorage`
  key). A window that receives a change doesn't re-persist it.
- Backend-owned data (library, cloud accounts, YouTube Music sign-in, account
  session) is not mirrored. The window that changed it emits a domain
  invalidation, and the main window re-runs the same refresh it runs today.

## 3. Equalizer mode switch

- The mode **is** `useSystemEqStore.enabled` (persisted intent, restored on
  launch). A second flag could disagree with the engine. One source of truth means
  the UI can never say "Equalizer" while nothing is equalizing.
- A `Player | Equalizer` segmented switch sits in the top bar in both modes:
  - → Equalizer: `requirePremium("SYSTEM_EQ")`, then `systemAudioStatus()`. If
    system audio is available, `enable()`; on success the shell switches. If this
    machine needs the one-time setup (Windows driver/APO, Linux component),
    Settings opens at `system-audio`, where that setup lives. Failures surface
    as today (toast plus the store's `error`).
  - → Player: `disable()`.
- The Settings window's Enable/Disable buttons drive the same store, which is
  synced, so the main window follows.

## 4. Equalizer mode shell

- Visible routes in Equalizer mode: Equalizer, Enhancer, Stems, Visualizers, in
  that order. Entering the mode lands on Equalizer. If the current route is hidden,
  it falls back to Equalizer (Player mode: Library).
- Hidden in Equalizer mode: Library, Explore, Stations, Movies, Mixer, the
  now-playing bar, the right sidebar (queue/lyrics/video), and the sidebar
  now-playing card. The app's own player isn't the source in this mode.
- Kept: the top bar (title, mode switch, waveform, master volume, meters, power,
  update pill, settings gear) and the update banner.
- **System audio panel** at the bottom of the sidebar, where now-playing sits in
  Player mode: live status (Active / Recovering / Off with **Restart**, polled
  from `systemEqStatus()` as Settings does), the scope summary, and **Choose apps…**
  (macOS per-app scope, the same actions as the Settings card), so the mode's own
  controls are one click away.
- **Effects rack** at the top of Enhancer: one chip per effect (Presets, Bass,
  Surround, 3D Surround, Room, Convolver, Compander, Saturation, Script, Headphone,
  Limiter) with an on/off switch and a jump to that effect's card. The switches
  call the same handlers the cards use (one shared hook, the same premium gates),
  so every effect can be toggled or reached in one click. Shown in both modes.
- Stems keeps its own behaviour. With no app track playing it already explains
  what it needs; in Equalizer mode that empty state adds the one-line reason
  (stems separate tracks HypeMuzik plays) and a **Switch to Player** action.

## Testing

- vitest: `windowSync` (diff, echo guard, merge, snapshot, ignore own label),
  `noticeFor`, mode-filtered routes and the route fallback, the mode switch
  (available / needs setup / enable fails / premium refused), the effects-rack hook
  against the card handlers.
- Rust: `cargo check`, `cargo test -p hypemuzik --lib` (updater status
  serialization with `version`).
- `pnpm build` (tsc + vite), `pnpm test`, `cargo clippy` as CI runs it.
- In the real app (`pnpm tauri dev`), with screenshots: the Settings window has only
  close; it opens with ⌘, and the gear; theme, EQ bands, visualizer and playback
  changes in Settings show in the main window at once and the reverse; the mode
  switch both ways; Equalizer-mode navigation; the update banner (driven by a
  simulated status in dev).
