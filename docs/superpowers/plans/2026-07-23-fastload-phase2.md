# Fast-Load Phase 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Relaunch-and-play within the ~6h URL window costs ~0.3s (probe) instead of ~5.3s (yt-dlp resolve); consecutive tracks reuse one HTTP client's connections instead of a fresh TLS handshake each.

**Architecture:** (R4) `YtMusicState` gains a quarantined `restored` map + generation counter + JSON snapshot/restore API; disk-restored entries are probed (`Range: bytes=0-1`) before first use — promoted into the live map on 200/206, dropped otherwise. src-tauri owns the file (same dir + write-then-rename pattern as `YtLibraryCache`), restoring at startup and saving on a 60s generation-gated interval plus exit. (R7) One shared `reqwest::blocking::Client` per crate via `OnceLock`.

**Tech Stack:** Rust (reqwest blocking + rustls, serde, OnceLock), Tauri 2 (setup + RunEvent), cargo test with real-TCP wire tests.

**Spec:** `docs/superpowers/specs/2026-07-23-fastload-phase2-design.md`
**Analysis:** `.superpowers/sdd/fable-fastload-analysis.md` §R4, §R7

## Global Constraints

- Branch: `feat/fastload-phase2` (off main; Phase 1 already merged).
- Probe rules EXACT: `GET` with `Range: bytes=0-1` + the entry's own headers; 200/206 → promote; any other status, error, or ~5s timeout → drop and fall through to a normal resolve. Probe ONLY disk-restored entries — same-session cache hits stay unprobed (~µs).
- Snapshot = fresh(live) ∪ fresh(restored), live wins on key conflict. Generation bumps on `remember()` and on probe-drop; NOT on restore, NOT on promotion.
- Envelope `{ "version": 1, "entries": { ... } }`; any parse failure or version mismatch is silently ignored (it is only a cache). Audio map only — the video map stays memory-only.
- File: `ytmusic-stream-urls.json`, same directory as the YtLibraryCache file, write-then-rename.
- hm-ytmusic's new `reqwest` dep uses `default-features = false, features = ["blocking", "rustls-tls"]` (workspace TLS choice — no native-tls/openssl creep).
- Nothing on the play path may panic; all cache I/O failures degrade to "no cache".
- Repo rules: no `Co-Authored-By`; push only at the end. Run all commands from repo root.

---

### Task 1: R4 core — snapshot/restore + generation on `YtMusicState`

**Files:**
- Modify: `crates/hm-ytmusic/src/ytdlp.rs` (`StreamTarget` derives, ~line 115)
- Modify: `crates/hm-ytmusic/src/lib.rs` (state fields ~line 175/256, helpers near `remember` ~line 699, tests in the existing `#[cfg(test)]` module)

**Interfaces:**
- Produces (used by Tasks 2–3): field `restored: TargetCache`; `pub fn url_cache_snapshot(&self) -> Option<(u64, String)>`; `pub fn restore_url_cache(&self, json: &str)`; `pub fn url_cache_generation(&self) -> u64`; internal `cache_generation: AtomicU64`.

- [ ] **Step 1: Serde on `StreamTarget`**

In `ytdlp.rs`, change the derive to:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StreamTarget {
```

(Internal file format — no rename attribute.)

- [ ] **Step 2: Write the failing tests**

In `lib.rs`'s test module (in-crate, so private fields/fns are reachable). These tests define the whole contract:

```rust
    fn target(expires_in: i64) -> ytdlp::StreamTarget {
        let now = now_secs().unwrap() as i64;
        ytdlp::StreamTarget {
            url: "https://cdn.example/x".into(),
            headers: vec![("User-Agent".into(), "hm".into())],
            ext: "m4a".into(),
            format_id: "140".into(),
            abr_kbps: Some(129),
            expires_at: (now + expires_in > 0).then(|| (now + expires_in) as u64),
        }
    }

    /// The whole point of persistence: what one session remembers, the next
    /// session can restore — and still-fresh means fresh on BOTH trips.
    #[test]
    fn url_cache_round_trips_fresh_entries() {
        let a = YtMusicState::new();
        a.remember("vid1", &target(6 * 3600));
        a.remember("vid2", &target(60)); // inside EXPIRY_MARGIN — not fresh
        let (_, json) = a.url_cache_snapshot().expect("one fresh entry to save");

        let b = YtMusicState::new();
        b.restore_url_cache(&json);
        let restored = b.restored.read().unwrap();
        assert!(restored.contains_key("vid1"), "the fresh entry must round-trip");
        assert!(!restored.contains_key("vid2"), "a near-expiry entry is not worth restoring");
        // Restored entries are quarantined, not live: the play path must probe
        // them first (IP-bound urls), so cached_target must NOT serve them.
        drop(restored);
        assert!(b.cached_target("vid1").is_none());
    }

    #[test]
    fn snapshot_is_the_union_of_live_and_restored() {
        let a = YtMusicState::new();
        a.remember("live1", &target(6 * 3600));
        let (_, json) = a.url_cache_snapshot().unwrap();

        let b = YtMusicState::new();
        b.restore_url_cache(&json);
        b.remember("live2", &target(6 * 3600));
        let (_, json2) = b.url_cache_snapshot().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(&json2).unwrap();
        let entries = envelope.pointer("/entries").unwrap().as_object().unwrap();
        // Without the union, every relaunch would shrink the file to only
        // what got played that session.
        assert!(entries.contains_key("live1"), "an unprobed restored entry must persist");
        assert!(entries.contains_key("live2"));
    }

    #[test]
    fn garbage_and_wrong_versions_are_ignored_not_fatal() {
        let s = YtMusicState::new();
        s.restore_url_cache("not json at all");
        s.restore_url_cache("{\"version\": 99, \"entries\": {}}");
        s.restore_url_cache("{}");
        assert!(s.restored.read().unwrap().is_empty());
        assert!(s.url_cache_snapshot().is_none(), "nothing restorable means nothing to save");
    }

    /// The saver polls the generation to skip no-op writes.
    #[test]
    fn generation_moves_on_remember_not_on_read_or_restore() {
        let s = YtMusicState::new();
        let g0 = s.url_cache_generation();
        let _ = s.cached_target("vid1");
        assert_eq!(s.url_cache_generation(), g0, "a read must not dirty the cache");
        s.remember("vid1", &target(6 * 3600));
        let g1 = s.url_cache_generation();
        assert!(g1 > g0, "a write must dirty the cache");
        let (_, json) = s.url_cache_snapshot().unwrap();
        let t = YtMusicState::new();
        t.restore_url_cache(&json);
        assert_eq!(
            t.url_cache_generation(),
            0,
            "restoring what came FROM the file must not schedule a rewrite of it"
        );
    }
```

Run: `cargo test -p hm-ytmusic url_cache 2>&1 | tail -5` → COMPILE ERROR (no such methods/field).

- [ ] **Step 3: Implement**

State struct additions (next to `resolved`):

```rust
    /// Disk-restored urls, quarantined until probed.
    ///
    /// A restart often means a network change, and googlevideo urls are
    /// IP-bound — so nothing in here may be served without one cheap probe
    /// first (see `live_or_probed_target`). Same-session entries never pass
    /// through this map and keep their unprobed ~µs hits.
    restored: TargetCache,
    /// Bumped whenever what a snapshot would contain changes — `remember`, and
    /// a probe dropping a restored entry. The disk saver polls it to skip
    /// writes when nothing moved. Restore doesn't bump: that state came FROM
    /// the file.
    cache_generation: std::sync::atomic::AtomicU64,
```

(Initialize both in `Self::new()`: `restored: RwLock::new(std::collections::HashMap::new())`, `cache_generation: std::sync::atomic::AtomicU64::new(0)`.)

Envelope + API (near `remember`):

```rust
/// On-disk shape of the persisted url cache. Versioned so a future
/// `StreamTarget` change can't half-parse an old file into wrong urls.
#[derive(serde::Serialize, serde::Deserialize)]
struct UrlCacheFile {
    version: u32,
    entries: std::collections::HashMap<String, ytdlp::StreamTarget>,
}

const URL_CACHE_VERSION: u32 = 1;
```

```rust
    /// The audio url cache as a JSON envelope, with its generation — or `None`
    /// when there is nothing fresh worth writing.
    ///
    /// The union of the live map and the not-yet-probed restored map (live
    /// wins): snapshotting only the live map would shrink the file to what got
    /// played this session, throwing away restored entries that are still
    /// perfectly probeable tomorrow.
    pub fn url_cache_snapshot(&self) -> Option<(u64, String)> {
        let now = now_secs()?;
        let mut entries: std::collections::HashMap<String, ytdlp::StreamTarget> = self
            .restored
            .read()
            .ok()?
            .iter()
            .filter(|(_, t)| is_fresh(t, now))
            .map(|(k, t)| (k.clone(), t.clone()))
            .collect();
        for (k, t) in self.resolved.read().ok()?.iter() {
            if is_fresh(t, now) {
                entries.insert(k.clone(), t.clone());
            }
        }
        if entries.is_empty() {
            return None;
        }
        let file = UrlCacheFile { version: URL_CACHE_VERSION, entries };
        let generation = self.url_cache_generation();
        serde_json::to_string(&file).ok().map(|json| (generation, json))
    }

    /// Load a previous session's url cache into quarantine.
    ///
    /// Tolerant by design — a cache that can't be read is a cache that doesn't
    /// exist, never an error: garbage, an old version, or a clock problem all
    /// just mean starting cold.
    pub fn restore_url_cache(&self, json: &str) {
        let Ok(file) = serde_json::from_str::<UrlCacheFile>(json) else {
            return;
        };
        if file.version != URL_CACHE_VERSION {
            return;
        }
        let Some(now) = now_secs() else { return };
        let Ok(mut restored) = self.restored.write() else {
            return;
        };
        for (id, t) in file.entries {
            if is_fresh(&t, now) {
                restored.insert(id, t);
            }
        }
    }

    pub fn url_cache_generation(&self) -> u64 {
        self.cache_generation.load(std::sync::atomic::Ordering::Relaxed)
    }
```

And in the existing `remember()` (audio path, ~line 699), add after `remember_in(...)`:

```rust
        self.cache_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
```

- [ ] **Step 4: Run**

`cargo test -p hm-ytmusic url_cache 2>&1 | tail -5` → 4 passed. Then the crate suite + clippy: `cargo test -p hm-ytmusic 2>&1 | tail -3 && cargo clippy -p hm-ytmusic --all-targets 2>&1 | tail -3` (a dead-code warning on `restored`/API until Tasks 2–3 consume them is acceptable ONLY if clippy stays quiet — the pub API and test usage should keep it quiet; report if not).

- [ ] **Step 5: Commit**

```bash
git add crates/hm-ytmusic/src/ytdlp.rs crates/hm-ytmusic/src/lib.rs
git commit -m "feat(ytmusic): persistable url cache with quarantined restore"
```

---

### Task 2: R4 probe — validate restored entries before first use

**Files:**
- Modify: `crates/hm-ytmusic/Cargo.toml` (add reqwest)
- Modify: `crates/hm-ytmusic/src/lib.rs` (probe + integration into `stream_target`/`prefetch`; wire tests)

**Interfaces:**
- Consumes: Task 1's `restored` map + generation.
- Produces: internal `fn live_or_probed_target(&self, video_id: &str) -> Option<ytdlp::StreamTarget>` — the ONLY cache read the resolve paths use from now on.

- [ ] **Step 1: Dependency**

`crates/hm-ytmusic/Cargo.toml`, next to the other deps:

```toml
# Probing disk-restored stream urls (one Range: bytes=0-1 GET before first
# use — the urls are IP-bound). Blocking to match the sync resolve path;
# rustls to match the workspace TLS choice.
reqwest = { version = "0.12", default-features = false, features = ["blocking", "rustls-tls"] }
```

(If the workspace `Cargo.toml` already declares a reqwest workspace dep — check `grep -n "reqwest" Cargo.toml` at the root — use `reqwest = { workspace = true, features = ["blocking"] }` instead, matching however hm-audio declares it.)

- [ ] **Step 2: Write the failing wire tests**

In `lib.rs`'s test module. The fake server is a plain `TcpListener` (same pattern as hm-audio's wire tests):

```rust
    /// One request, canned response, captured request lines.
    fn one_shot_server(
        status_line: &'static str,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut lines = Vec::new();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line == "\r\n" || line == "\n" {
                    break;
                }
                lines.push(line.trim_end().to_string());
            }
            let mut w = stream;
            let _ = write!(w, "{status_line}\r\nContent-Length: 2\r\n\r\nok");
            lines
        });
        (format!("http://{addr}/probe"), handle)
    }

    fn restored_state_with(url: String) -> YtMusicState {
        let s = YtMusicState::new();
        let now = now_secs().unwrap();
        let t = ytdlp::StreamTarget {
            url,
            headers: vec![("User-Agent".into(), "hm-probe-test".into())],
            ext: "m4a".into(),
            format_id: "140".into(),
            abr_kbps: None,
            expires_at: Some(now + 6 * 3600),
        };
        s.restored.write().unwrap().insert("vid1".into(), t);
        s
    }

    /// A restored entry that answers is promoted: served now, live (unprobed)
    /// forever after — and the probe itself asks for two bytes, not the track.
    #[test]
    fn a_healthy_restored_entry_is_probed_once_then_live() {
        let (url, server) = one_shot_server("HTTP/1.1 206 Partial Content");
        let s = restored_state_with(url.clone());
        let got = s.live_or_probed_target("vid1").expect("a 206 probe must serve the entry");
        assert_eq!(got.url, url);
        let lines = server.join().unwrap();
        assert!(
            lines.iter().any(|l| l.eq_ignore_ascii_case("range: bytes=0-1")),
            "the probe must ask for two bytes, not the body; got {lines:#?}"
        );
        assert!(
            lines.iter().any(|l| l.eq_ignore_ascii_case("user-agent: hm-probe-test")),
            "the entry's own headers must go out — the CDN checks them"
        );
        // Promoted: second read is a live hit, no second request (the one-shot
        // server is already gone, so a re-probe would return None here).
        assert_eq!(s.live_or_probed_target("vid1").unwrap().url, got.url);
        assert!(s.restored.read().unwrap().is_empty(), "quarantine is over");
    }

    /// 403 is what an IP-bound url looks like from a new network: the entry is
    /// dead on arrival — drop it so the caller falls through to a fresh
    /// resolve, and dirty the cache so the dead entry leaves the file too.
    #[test]
    fn a_dead_restored_entry_is_dropped_and_dirties_the_cache() {
        let (url, server) = one_shot_server("HTTP/1.1 403 Forbidden");
        let s = restored_state_with(url);
        let g0 = s.url_cache_generation();
        assert!(s.live_or_probed_target("vid1").is_none());
        let _ = server.join();
        assert!(s.restored.read().unwrap().is_empty());
        assert!(s.cached_target("vid1").is_none());
        assert!(s.url_cache_generation() > g0, "the union changed; the saver must notice");
    }

    /// Same-session entries never probe — the whole point of quarantining.
    #[test]
    fn a_live_entry_is_served_without_any_request() {
        let s = YtMusicState::new();
        let now = now_secs().unwrap();
        let t = ytdlp::StreamTarget {
            url: "http://127.0.0.1:1/unreachable".into(),
            headers: vec![],
            ext: "m4a".into(),
            format_id: "140".into(),
            abr_kbps: None,
            expires_at: Some(now + 6 * 3600),
        };
        s.remember("vid1", &t);
        assert_eq!(
            s.live_or_probed_target("vid1").expect("live hits must not probe").url,
            t.url
        );
    }
```

Run: `cargo test -p hm-ytmusic --no-run 2>&1 | tail -5` → COMPILE ERROR (`live_or_probed_target` missing).

- [ ] **Step 3: Implement**

Shared probe client + probe + gatekeeper (near `cached_target`):

```rust
/// One blocking client for every probe: connection reuse, and no per-probe
/// construction cost. Separate from hm-audio's stream client by crate
/// boundary — the probe's TLS warm doesn't transfer there; accepted, the
/// probe's job is validity, not warming.
fn probe_client() -> &'static reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("default TLS config must build")
    })
}

/// Whether `target` still answers from THIS network. Two bytes, ranged — the
/// cheapest question the CDN accepts (~100–300ms).
fn probe_ok(target: &ytdlp::StreamTarget) -> bool {
    let mut req = probe_client().get(&target.url).header("Range", "bytes=0-1");
    for (k, v) in &target.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    match req.send() {
        Ok(r) => {
            let s = r.status();
            s == reqwest::StatusCode::OK || s == reqwest::StatusCode::PARTIAL_CONTENT
        }
        Err(_) => false,
    }
}
```

On `impl YtMusicState`:

```rust
    /// The cache read every resolve path goes through: a live hit is served
    /// as-is (~µs); a disk-restored hit is probed first — promoted on 200/206,
    /// dropped otherwise so the caller falls through to a fresh resolve.
    fn live_or_probed_target(&self, video_id: &str) -> Option<ytdlp::StreamTarget> {
        if let Some(t) = self.cached_target(video_id) {
            return Some(t);
        }
        let quarantined = self.restored.write().ok()?.remove(video_id)?;
        if probe_ok(&quarantined) {
            // remember() also bumps the generation — promotion doesn't change
            // the snapshot union, but the bump is harmless (one spare write).
            self.remember(video_id, &quarantined);
            return Some(quarantined);
        }
        // Dead on arrival (new network, revoked url): the union changed, so
        // the saver must write the shrunken truth.
        self.cache_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        None
    }
```

Then switch the resolve paths over: in `stream_target` and `prefetch` (and any other audio-path caller of `cached_target` — `grep -n "cached_target" crates/hm-ytmusic/src/lib.rs`), replace the `cached_target(...)` read with `live_or_probed_target(...)`. Do NOT touch the video map's read path.

- [ ] **Step 4: Run**

`cargo test -p hm-ytmusic 2>&1 | tail -3 && cargo clippy -p hm-ytmusic --all-targets 2>&1 | tail -3` → all green, no warnings. Note: the Task 1 test `url_cache_round_trips_fresh_entries` asserts `cached_target` returns None for restored entries — it must STILL pass (the quarantine read is only via `live_or_probed_target`).

- [ ] **Step 5: Commit**

```bash
git add crates/hm-ytmusic/Cargo.toml crates/hm-ytmusic/src/lib.rs Cargo.lock
git commit -m "feat(ytmusic): probe disk-restored urls before first use"
```

---

### Task 3: R4 persistence — the file, the saver, the restore

**Files:**
- Modify: `src-tauri/src/ytmusic.rs` (file helpers next to `YtLibraryCache`)
- Modify: `src-tauri/src/lib.rs` (restore at ~line 433 after `YtMusicState::load()`; saver task; exit save at the `RunEvent::ExitRequested` arm ~line 804)

**Interfaces:**
- Consumes: `url_cache_snapshot()`, `restore_url_cache()`, `url_cache_generation()` (Tasks 1–2).

- [ ] **Step 1: File helpers**

In `src-tauri/src/ytmusic.rs` (same file as `YtLibraryCache`, reusing its conventions):

```rust
/// Where the persisted stream-url cache lives: beside the library cache.
pub fn url_cache_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    // Same directory the YtLibraryCache file is built from (see setup in
    // lib.rs) — one place for all YT Music disk state.
    use tauri::Manager;
    app.path().app_data_dir().ok().map(|d| {
        let _ = std::fs::create_dir_all(&d);
        d.join("ytmusic-stream-urls.json")
    })
}

/// Write-then-rename, like the library cache: a crash mid-write can't leave a
/// half-parsed file — and a half-parsed cache is silently ignored anyway.
pub fn save_url_cache(path: &std::path::Path, json: &str) {
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(tmp, path);
    }
}
```

(Adjust `url_cache_path` to mirror EXACTLY how the `yt_lib_path` is derived in `lib.rs` around line 460 — same base dir call. If the library cache uses a different dir helper, match it and say so in the report.)

- [ ] **Step 2: Restore at startup + the saver task**

In `src-tauri/src/lib.rs`, right after `app.manage(hm_ytmusic::YtMusicState::load());` (~line 433):

```rust
            // Yesterday's stream urls are good for ~6 hours; restoring them
            // makes relaunch-and-play cost one ~300ms probe instead of a ~5s
            // yt-dlp resolve. Quarantined until probed — see hm-ytmusic.
            if let Some(path) = ytmusic::url_cache_path(app.handle()) {
                if let Ok(json) = std::fs::read_to_string(&path) {
                    app.state::<hm_ytmusic::YtMusicState>().restore_url_cache(&json);
                }
                // Save on a slow heartbeat, only when something changed. The
                // entries are worth at most ~6h, so losing the tail on a crash
                // costs one resolve — no need for write-on-every-change.
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut last_saved: u64 = 0;
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        let state = handle.state::<hm_ytmusic::YtMusicState>();
                        let generation = state.url_cache_generation();
                        if generation == last_saved {
                            continue;
                        }
                        if let Some((g, json)) = state.url_cache_snapshot() {
                            ytmusic::save_url_cache(&path, &json);
                            last_saved = g;
                        }
                    }
                });
            }
```

(If `tokio` isn't already an import path available in src-tauri, use `tauri::async_runtime`'s sleep equivalent or add the crate import matching how other periodic tasks in this file sleep — `grep -n "sleep" src-tauri/src/lib.rs` and match precedent. Report which.)

- [ ] **Step 3: Exit save**

In the `run(...)` event closure, inside the `RunEvent::ExitRequested` arm (next to `updater::install_on_exit`):

```rust
                // Flush the stream-url cache: the 60s heartbeat may owe a write.
                if let Some(path) = ytmusic::url_cache_path(_app) {
                    if let Some((_, json)) =
                        _app.state::<hm_ytmusic::YtMusicState>().url_cache_snapshot()
                    {
                        ytmusic::save_url_cache(&path, &json);
                    }
                }
```

(Match the closure's actual handle variable name — the arm already uses `_app`.)

- [ ] **Step 4: Verify**

`cargo check --workspace 2>&1 | tail -3 && cargo clippy --workspace --all-targets 2>&1 | tail -3` → clean (only the pre-existing `block v0.1.6` note).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ytmusic.rs src-tauri/src/lib.rs
git commit -m "feat(ytmusic): persist the stream-url cache across launches"
```

---

### Task 4: R7 — shared blocking client in hm-audio

**Files:**
- Modify: `crates/hm-audio/src/streaming.rs` (client construction ~line 581)
- Modify: `crates/hm-audio/src/stream_queue.rs` (client construction ~lines 73-78)

**Interfaces:**
- Produces: `pub(crate) fn shared_client() -> &'static reqwest::blocking::Client` in `streaming.rs`, used by both files.

- [ ] **Step 1: The shared client**

In `streaming.rs`, near the top-level helpers:

```rust
/// One blocking client for every stream in the process.
///
/// A fresh client per stream re-did the TLS handshake for every track — on
/// the same googlevideo host, back to back, ~100-300ms a time. reqwest pools
/// connections per client, so sharing one is what makes consecutive tracks
/// reuse the socket. Config matches what both call sites built individually.
pub(crate) fn shared_client() -> &'static reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(12))
            .build()
            .expect("default TLS config must build")
    })
}
```

FIRST read both existing construction sites (`streaming.rs` ~581, `stream_queue.rs` ~73-78). If their builder configs differ from each other (timeouts, redirect policy, user-agent), STOP and report the difference instead of unifying silently — the shared config must be the superset both paths accept. If they are identical (or one is a subset), use that config and note it.

- [ ] **Step 2: Switch both sites**

Replace each site's `Client::builder()...build()` (and any surrounding `Ok`/`?` plumbing) with `shared_client()` (in `stream_queue.rs`: `streaming::shared_client()` or via the crate-internal path that matches the module tree — check how stream_queue already references streaming items).

- [ ] **Step 3: Verify**

`cargo test -p hm-audio 2>&1 | tail -3 && cargo clippy -p hm-audio --all-targets 2>&1 | tail -3` → 107 tests green (the wire tests build their own clients and are unaffected), no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/hm-audio/src/streaming.rs crates/hm-audio/src/stream_queue.rs
git commit -m "perf(streaming): one shared HTTP client — reuse connections across tracks"
```

---

### Task 5: Verification + push

- [ ] **Step 1: Whole-workspace gates**

```bash
cargo clippy --workspace --all-targets 2>&1 | tail -3
cargo test -p hm-audio -p hm-ytmusic -p hm-core 2>&1 | grep "test result"
pnpm exec tsc --noEmit 2>&1 | tail -3
pnpm test -- --run 2>&1 | tail -4
pnpm build 2>&1 | tail -2
```

All green (hm-remote iroh = environmental, skip).

- [ ] **Step 2: Push**

```bash
git push -u origin feat/fastload-phase2
```

- [ ] **Step 3: Memory**

Update `~/.claude/projects/-Users-bruno-me-COTE/memory/hypemuzik_desktop_fastload.md`: Phase 2 implemented (commits, what shipped, NOT device-tested; add "relaunch-and-play + probe path" to the manual checklist).
