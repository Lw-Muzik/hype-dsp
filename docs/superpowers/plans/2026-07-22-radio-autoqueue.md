# Endless Radio Auto-Queue Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When the user plays a song from YT Music search or an Explore shelf, the queue becomes that song plus an endless, continuously replenished stream of similar tracks from YouTube Music's own radio algorithm; YT playlist/album/artist queues extend the same way when they finish.

**Architecture:** A new hand-parsed `radio.rs` in `hm-ytmusic` calls the InnerTube `next` endpoint (`GetWatchPlaylistQuery`, radio playlist `RDAMVM<videoId>`) and follows `nextRadioContinuationData` tokens. Two Tauri commands expose page-1 and continuation fetches. The frontend engine store owns the radio session (module-level, like `gaplessQueueRunning`) and appends deduped batches whenever ≤5 unplayed tracks remain ahead, gated by a persisted `autoplay` flag on `PlaybackState`.

**Tech Stack:** Rust (ytmapi-rs 0.3.2, serde_json), Tauri 2, TypeScript + Zustand + React 19, vitest, cargo test.

**Spec:** `docs/superpowers/specs/2026-07-22-radio-autoqueue-design.md`

## Global Constraints

- The `next` endpoint takes RAW playlist ids — NO `VL` prefix (the opposite of `browse`).
- Nothing in the Rust parser may hard-fail: an unreadable row is skipped, never fatal (same rule as `explore.rs` / `playlist.rs`).
- Radio fetches never interrupt playback: failures are console-logged, no toasts.
- Autoplay defaults ON; the flag persists via the existing EngineState autosave.
- Repo rules: no `Co-Authored-By` lines in commits; push after committing.
- Run all commands from repo root `~/me/COTE/hypemuzik-desktop`.

---

### Task 1: Rust radio-page parser (`radio.rs`)

**Files:**
- Create: `crates/hm-ytmusic/src/radio.rs`
- Create: `crates/hm-ytmusic/tests/fixtures/radio_page.json`
- Create: `crates/hm-ytmusic/tests/fixtures/radio_continuation.json`
- Modify: `crates/hm-ytmusic/src/lib.rs` (add `mod radio;` + `pub use radio::RadioBatch;` next to the existing `mod`/`pub use` lines near the top)

**Interfaces:**
- Consumes: `crate::nav::{best_thumbnail, join_runs}`, `crate::{parse_duration, YtTrack}` (all existing).
- Produces: `pub struct RadioBatch { pub tracks: Vec<YtTrack>, pub continuation: Option<String> }` (serde camelCase) and `pub(crate) fn parse_radio_page(json: &Value) -> RadioBatch` — used by Task 2.

- [ ] **Step 1: Write the fixtures**

`crates/hm-ytmusic/tests/fixtures/radio_page.json` — a trimmed but structurally real first page. Note: row 1 is the seed (`selected: true`), row 2 is a wrapper renderer (with a counterpart that must NOT become a second track), row 3 is plain, row 4 is unplayable, row 5 is malformed (no videoId):

```json
{
  "contents": {
    "singleColumnMusicWatchNextResultsRenderer": {
      "tabbedRenderer": {
        "watchNextTabbedResultsRenderer": {
          "tabs": [
            {
              "tabRenderer": {
                "content": {
                  "musicQueueRenderer": {
                    "content": {
                      "playlistPanelRenderer": {
                        "playlistId": "RDAMVMseed00000",
                        "isInfinite": true,
                        "numItemsToShow": 25,
                        "contents": [
                          {
                            "playlistPanelVideoRenderer": {
                              "videoId": "seed00000",
                              "selected": true,
                              "title": { "runs": [{ "text": "Seed Song" }] },
                              "longBylineText": { "runs": [{ "text": "Seed Artist" }] },
                              "lengthText": { "runs": [{ "text": "3:00" }] },
                              "thumbnail": { "thumbnails": [{ "url": "https://i/seed.jpg", "width": 60, "height": 60 }] }
                            }
                          },
                          {
                            "playlistPanelVideoWrapperRenderer": {
                              "primaryRenderer": {
                                "playlistPanelVideoRenderer": {
                                  "videoId": "wrapped01",
                                  "title": { "runs": [{ "text": "Wrapped Song" }] },
                                  "longBylineText": {
                                    "runs": [
                                      { "text": "Artist A", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCabc" } } },
                                      { "text": " • " },
                                      { "text": "Album A", "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_a1" } } },
                                      { "text": " • " },
                                      { "text": "2021" }
                                    ]
                                  },
                                  "lengthText": { "runs": [{ "text": "3:32" }] },
                                  "thumbnail": { "thumbnails": [{ "url": "https://i/w1_small.jpg", "width": 60, "height": 60 }, { "url": "https://i/w1.jpg", "width": 544, "height": 544 }] },
                                  "navigationEndpoint": {
                                    "watchEndpoint": {
                                      "videoId": "wrapped01",
                                      "playlistId": "RDAMVMseed00000",
                                      "watchEndpointMusicSupportedConfigs": { "watchEndpointMusicConfig": { "musicVideoType": "MUSIC_VIDEO_TYPE_ATV" } }
                                    }
                                  }
                                }
                              },
                              "counterpart": [
                                {
                                  "counterpartRenderer": {
                                    "playlistPanelVideoRenderer": {
                                      "videoId": "wrapped01",
                                      "title": { "runs": [{ "text": "Wrapped Song (Video)" }] },
                                      "lengthText": { "runs": [{ "text": "3:35" }] },
                                      "navigationEndpoint": {
                                        "watchEndpoint": {
                                          "videoId": "wrapped01",
                                          "watchEndpointMusicSupportedConfigs": { "watchEndpointMusicConfig": { "musicVideoType": "MUSIC_VIDEO_TYPE_OMV" } }
                                        }
                                      }
                                    }
                                  }
                                }
                              ]
                            }
                          },
                          {
                            "playlistPanelVideoRenderer": {
                              "videoId": "plain0002",
                              "title": { "runs": [{ "text": "Plain Video" }] },
                              "longBylineText": { "runs": [{ "text": "Artist B" }, { "text": " • " }, { "text": "14M views" }] },
                              "lengthText": { "runs": [{ "text": "4:05" }] },
                              "thumbnail": { "thumbnails": [{ "url": "https://i/p2.jpg", "width": 400, "height": 225 }] },
                              "navigationEndpoint": {
                                "watchEndpoint": {
                                  "videoId": "plain0002",
                                  "playlistId": "RDAMVMseed00000",
                                  "watchEndpointMusicSupportedConfigs": { "watchEndpointMusicConfig": { "musicVideoType": "MUSIC_VIDEO_TYPE_OMV" } }
                                }
                              }
                            }
                          },
                          {
                            "playlistPanelVideoRenderer": {
                              "videoId": "blocked03",
                              "unplayableText": { "runs": [{ "text": "Not available" }] },
                              "title": { "runs": [{ "text": "Region Blocked" }] }
                            }
                          },
                          {
                            "playlistPanelVideoRenderer": {
                              "title": { "runs": [{ "text": "No Video Id" }] }
                            }
                          }
                        ],
                        "continuations": [
                          { "nextRadioContinuationData": { "continuation": "TOKEN_PAGE_2" } }
                        ]
                      }
                    }
                  }
                }
              }
            }
          ]
        }
      }
    }
  }
}
```

`crates/hm-ytmusic/tests/fixtures/radio_continuation.json` — the continuation shape (`continuationContents.playlistPanelContinuation`), one plain row, its own token:

```json
{
  "continuationContents": {
    "playlistPanelContinuation": {
      "contents": [
        {
          "playlistPanelVideoRenderer": {
            "videoId": "cont00001",
            "title": { "runs": [{ "text": "Continuation Song" }] },
            "longBylineText": { "runs": [{ "text": "Artist C" }] },
            "lengthText": { "runs": [{ "text": "2:58" }] },
            "thumbnail": { "thumbnails": [{ "url": "https://i/c1.jpg", "width": 544, "height": 544 }] },
            "navigationEndpoint": {
              "watchEndpoint": {
                "videoId": "cont00001",
                "playlistId": "RDAMVMseed00000",
                "watchEndpointMusicSupportedConfigs": { "watchEndpointMusicConfig": { "musicVideoType": "MUSIC_VIDEO_TYPE_ATV" } }
              }
            }
          }
        }
      ],
      "continuations": [
        { "nextRadioContinuationData": { "continuation": "TOKEN_PAGE_3" } }
      ]
    }
  }
}
```

- [ ] **Step 2: Write the failing tests**

Create `crates/hm-ytmusic/src/radio.rs` with ONLY the test module first (types/fn referenced don't exist yet):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> serde_json::Value {
        serde_json::from_str(include_str!("../tests/fixtures/radio_page.json")).unwrap()
    }
    fn continuation() -> serde_json::Value {
        serde_json::from_str(include_str!("../tests/fixtures/radio_continuation.json")).unwrap()
    }

    #[test]
    fn parses_the_rows_and_the_token() {
        let batch = parse_radio_page(&page());
        assert_eq!(batch.continuation.as_deref(), Some("TOKEN_PAGE_2"));
        let ids: Vec<&str> = batch.tracks.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!(ids, vec!["wrapped01", "plain0002"]);
    }

    /// The seed comes back as contents[0] with `selected: true`; queueing it
    /// again would play the song the user just heard.
    #[test]
    fn skips_the_selected_seed_row() {
        let batch = parse_radio_page(&page());
        assert!(batch.tracks.iter().all(|t| t.video_id != "seed00000"));
    }

    /// A wrapper's counterpart is the same track in its other rendition —
    /// reading both would duplicate it in the queue.
    #[test]
    fn reads_a_wrapper_once_not_twice() {
        let batch = parse_radio_page(&page());
        let n = batch.tracks.iter().filter(|t| t.video_id == "wrapped01").count();
        assert_eq!(n, 1);
        let t = batch.tracks.iter().find(|t| t.video_id == "wrapped01").unwrap();
        assert_eq!(t.title, "Wrapped Song");
        assert!(!t.has_video, "the primary rendition is ATV — audio, not video");
    }

    #[test]
    fn maps_the_byline_album_by_its_album_link_not_by_position() {
        let batch = parse_radio_page(&page());
        let t = batch.tracks.iter().find(|t| t.video_id == "wrapped01").unwrap();
        assert_eq!(t.artist.as_deref(), Some("Artist A"));
        assert_eq!(t.album.as_deref(), Some("Album A"));
        // "14M views" is plain text, not an album link — must stay out of `album`.
        let v = batch.tracks.iter().find(|t| t.video_id == "plain0002").unwrap();
        assert_eq!(v.album, None);
        assert!(v.has_video, "OMV is a real music video");
    }

    #[test]
    fn skips_unplayable_and_malformed_rows_without_failing_the_page() {
        let batch = parse_radio_page(&page());
        assert!(batch.tracks.iter().all(|t| t.video_id != "blocked03"));
        assert_eq!(batch.tracks.len(), 2, "the two good rows must survive the bad ones");
    }

    #[test]
    fn fills_the_track_fields_the_queue_needs() {
        let batch = parse_radio_page(&page());
        let t = batch.tracks.iter().find(|t| t.video_id == "wrapped01").unwrap();
        assert_eq!(t.duration_secs, Some(212.0));
        assert_eq!(t.thumbnail.as_deref(), Some("https://i/w1.jpg"));
        assert_eq!(t.playlist_id, "RDAMVMseed00000");
        assert_eq!(t.playlist_title, "Radio");
        assert!(t.is_available);
    }

    #[test]
    fn parses_a_continuation_page_with_the_same_parser() {
        let batch = parse_radio_page(&continuation());
        assert_eq!(batch.tracks.len(), 1);
        assert_eq!(batch.tracks[0].video_id, "cont00001");
        assert_eq!(batch.continuation.as_deref(), Some("TOKEN_PAGE_3"));
    }

    #[test]
    fn a_page_with_no_token_reports_none_not_a_panic() {
        let batch = parse_radio_page(&serde_json::json!({ "contents": {} }));
        assert!(batch.tracks.is_empty());
        assert_eq!(batch.continuation, None);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p hm-ytmusic radio 2>&1 | tail -5`
Expected: COMPILE ERROR — `parse_radio_page` / `RadioBatch` not found.

- [ ] **Step 4: Write the parser**

Prepend to `crates/hm-ytmusic/src/radio.rs` (above the test module):

```rust
//! Radio ("up next") rows from the `next` endpoint's playlist panel.
//!
//! # Why this module owns a parser
//!
//! Upstream's watch-playlist parser collects rows with `Result`, so one odd
//! row destroys the page; it skips no `unplayableText` rows; and its track
//! model is too thin to queue (no album, no video type — the very field that
//! decides whether the Video tab is offered). Radio pages also wrap most rows
//! in `playlistPanelVideoWrapperRenderer` — a song/video counterpart pair, 42
//! of 50 rows in a real capture — and the counterpart must NOT be read as a
//! second track.
//!
//! Walking for the renderers rather than indexing a fixed path also makes this
//! shape-agnostic: the first page nests the panel under
//! `singleColumnMusicWatchNextResultsRenderer…musicQueueRenderer` and
//! continuation pages under `continuationContents.playlistPanelContinuation`,
//! and neither matters if you just look for the rows.
//!
//! **Nothing here may hard-fail** — an unreadable row is skipped, never fatal.
//! That is the whole point; see [`crate::explore`] for the same rule.

use crate::nav::{best_thumbnail, join_runs};
use crate::{parse_duration, YtTrack};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One page of a radio: the playable rows and the token for the next page.
/// Radio panels chain forever — a missing token is the exception, not the end
/// condition (the store re-seeds when it happens).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadioBatch {
    pub tracks: Vec<YtTrack>,
    pub continuation: Option<String>,
}

/// Reads a first page and a continuation page alike.
pub(crate) fn parse_radio_page(json: &Value) -> RadioBatch {
    let mut rows = Vec::new();
    collect_rows(json, &mut rows);
    RadioBatch {
        tracks: rows.iter().filter_map(|r| parse_row(r)).collect(),
        continuation: next_radio_token(json),
    }
}

/// Every track row in the panel, wrappers unwrapped to their primary rendition.
fn collect_rows<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(o) => {
            if let Some(w) = o.get("playlistPanelVideoWrapperRenderer") {
                // A wrapper is a song/video pair; the counterpart is the same
                // track in its other rendition, so only the primary is a row.
                if let Some(p) = w.pointer("/primaryRenderer/playlistPanelVideoRenderer") {
                    out.push(p);
                }
                return;
            }
            if let Some(r) = o.get("playlistPanelVideoRenderer") {
                out.push(r);
                return;
            }
            o.values().for_each(|v| collect_rows(v, out));
        }
        Value::Array(a) => a.iter().for_each(|v| collect_rows(v, out)),
        _ => {}
    }
}

/// One panel row → a queueable track; `None` for rows the queue must not hold.
fn parse_row(r: &Value) -> Option<YtTrack> {
    // Grey rows are listed but unplayable — a queue entry that can't stream.
    if r.pointer("/unplayableText").is_some() {
        return None;
    }
    // The seed comes back first, marked `selected` — the queue already has it.
    if r.pointer("/selected").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let video_id = r.pointer("/videoId").and_then(Value::as_str)?.to_string();
    let title = join_runs(r.pointer("/title/runs"))?;
    let byline = r.pointer("/longBylineText/runs").and_then(Value::as_array);
    // The byline reads "Artist • Album • 2021" — but the album is found by its
    // *link* (album browse ids start "MPRE"), not its position: videos put a
    // view count where songs put the album, and years/views are plain text.
    let artist = byline
        .and_then(|runs| runs.first())
        .and_then(|run| run.pointer("/text"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| join_runs(r.pointer("/shortBylineText/runs")));
    let album = byline.and_then(|runs| {
        runs.iter().skip(2).find_map(|run| {
            let id = run
                .pointer("/navigationEndpoint/browseEndpoint/browseId")
                .and_then(Value::as_str)?;
            if !id.starts_with("MPRE") {
                return None;
            }
            run.pointer("/text").and_then(Value::as_str).map(str::to_string)
        })
    });
    let duration_secs = r
        .pointer("/lengthText/runs/0/text")
        .and_then(Value::as_str)
        .and_then(parse_duration);
    let video_type = r
        .pointer("/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType")
        .and_then(Value::as_str);
    Some(YtTrack {
        video_id,
        title,
        artist,
        album,
        duration_secs,
        thumbnail: best_thumbnail(r.pointer("/thumbnail/thumbnails")),
        playlist_id: r
            .pointer("/navigationEndpoint/watchEndpoint/playlistId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        playlist_title: "Radio".into(),
        is_available: true,
        // ATV "videos" are a square still of the cover art; only OMV/UGC have
        // footage. Absent type defaults to no video — same call as the library.
        has_video: video_type.is_some_and(|t| t != "MUSIC_VIDEO_TYPE_ATV"),
    })
}

/// The token for the next radio page. Radio panels use
/// `nextRadioContinuationData` (plain queue panels use `nextContinuationData`,
/// which this deliberately does not follow — those queues are finite).
fn next_radio_token(json: &Value) -> Option<String> {
    fn walk(v: &Value) -> Option<String> {
        match v {
            Value::Object(o) => {
                if let Some(t) = o
                    .pointer("/nextRadioContinuationData/continuation")
                    .and_then(Value::as_str)
                {
                    return Some(t.to_string());
                }
                o.values().find_map(walk)
            }
            Value::Array(a) => a.iter().find_map(walk),
            _ => None,
        }
    }
    walk(json)
}
```

Then in `crates/hm-ytmusic/src/lib.rs`, next to the existing module declarations (`mod nav;` etc., near the top):

```rust
mod radio;
```

and next to the existing re-exports (where `ExploreSection` etc. are exported):

```rust
pub use radio::RadioBatch;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p hm-ytmusic radio 2>&1 | tail -5`
Expected: `test result: ok. 8 passed` (the 8 tests above).

Note: `duration_secs` asserts `Some(212.0)` — "3:32" = 212s via the existing `parse_duration`.

- [ ] **Step 6: Commit**

```bash
git add crates/hm-ytmusic/src/radio.rs crates/hm-ytmusic/src/lib.rs crates/hm-ytmusic/tests/fixtures/radio_page.json crates/hm-ytmusic/tests/fixtures/radio_continuation.json
git commit -m "feat(ytmusic): tolerant parser for radio (up-next) pages"
```

---

### Task 2: `YtMusicState::radio` + `radio_continue` (network layer)

**Files:**
- Modify: `crates/hm-ytmusic/src/lib.rs`

**Interfaces:**
- Consumes: `radio::parse_radio_page` (Task 1), existing `self.client()`, the `PlaylistContinuation`/`RawPage` pattern at `lib.rs:899-940`.
- Produces: `pub async fn radio(&self, video_id: &str) -> Result<RadioBatch, String>` and `pub async fn radio_continue(&self, video_id: &str, token: &str) -> Result<RadioBatch, String>` on `impl YtMusicState` — used by Task 3. NOTE: `radio_continue` takes the seed `video_id` too — the continuation re-POSTs the same body plus the token.

- [ ] **Step 1: Write the failing live test**

In the `#[cfg(test)]` module of `lib.rs`, next to `live_library_listing_parses` (~line 2110):

```rust
/// Run with `cargo test -p hm-ytmusic -- --ignored`.
#[tokio::test]
#[ignore = "requires a signed-in YT Music session in the keychain and network access"]
async fn live_radio_pages_endlessly() {
    let Some(state) = live_state() else {
        eprintln!("skipping: no session in the keychain — sign in through the app first");
        return;
    };
    // A track with a famously deep similarity graph.
    let seed = "dQw4w9WgXcQ";
    let first = state.radio(seed).await.expect("the radio page must parse");
    eprintln!("radio page 1: {} tracks", first.tracks.len());
    assert!(
        first.tracks.len() >= 10,
        "a radio should fill a queue, got {}",
        first.tracks.len()
    );
    assert!(
        first.tracks.iter().all(|t| t.video_id != seed),
        "the seed must not be re-queued"
    );
    let token = first.continuation.expect("radio pages must chain");
    let second = state
        .radio_continue(seed, &token)
        .await
        .expect("the continuation must parse");
    eprintln!("radio page 2: {} tracks", second.tracks.len());
    assert!(!second.tracks.is_empty(), "the continuation should keep the radio going");
    assert!(second.continuation.is_some(), "radio should always offer another page");
}
```

- [ ] **Step 2: Verify it fails to compile**

Run: `cargo test -p hm-ytmusic --no-run 2>&1 | tail -3`
Expected: COMPILE ERROR — no method `radio` on `YtMusicState`.

- [ ] **Step 3: Implement the two methods + the continuation query**

Add `GetWatchPlaylistQuery` to the existing `ytmapi_rs::query` import list (~line 53), and `VideoID` to the existing `ytmapi_rs::common` import (where `PlaylistID`, `ArtistChannelID` come from).

Inside `impl YtMusicState` (after `artist_page`):

```rust
/// The endless "up next" YT Music derives from one song — its radio.
///
/// This is the same `next` call the YT Music client makes: playlist
/// `RDAMVM<videoId>`, raw id, **no `VL` prefix** (`next` is the opposite of
/// `browse` on this). The typed parser upstream would hard-fail the page on
/// one odd row and skip no unplayable ones, so the JSON is read by
/// [`radio::parse_radio_page`] instead.
pub async fn radio(&self, video_id: &str) -> Result<RadioBatch, String> {
    let yt = self.client().await?;
    let query = GetWatchPlaylistQuery::new_from_video_id(VideoID::from_raw(video_id));
    let raw = yt
        .json_query(&query)
        .await
        .map_err(|e| format!("Radio unavailable: {e}"))?;
    let json: Value =
        ytmapi_rs::json::from_json(raw).map_err(|e| format!("Radio unreadable: {e}"))?;
    Ok(radio::parse_radio_page(&json))
}

/// The next page of a radio: the same body as [`Self::radio`] plus the token
/// the previous page returned. Needs the seed's `video_id` because the wire
/// format re-POSTs the full body — the token alone is not a request.
pub async fn radio_continue(&self, video_id: &str, token: &str) -> Result<RadioBatch, String> {
    let yt = self.client().await?;
    let base = GetWatchPlaylistQuery::new_from_video_id(VideoID::from_raw(video_id));
    let cont = WatchContinuation {
        base: &base,
        token: token.to_string(),
    };
    let raw = yt
        .json_query::<WatchContinuation>(&cont)
        .await
        .map_err(|e| format!("Radio continuation failed: {e}"))?;
    let json: Value =
        ytmapi_rs::json::from_json(raw).map_err(|e| format!("Radio unreadable: {e}"))?;
    Ok(radio::parse_radio_page(&json))
}
```

Below `PlaylistContinuation` (~line 940), its watch-panel twin:

```rust
/// One continuation of a radio: the same `next` POST as the first page, plus
/// the token. Exists for the same reason as [`PlaylistContinuation`] —
/// upstream's `GetContinuationsQuery` can only be built through the typed
/// parser this crate routes around. Wire format is upstream's own
/// (`ctoken` + `continuation`, same header and path).
struct WatchContinuation<'a> {
    base: &'a GetWatchPlaylistQuery<ytmapi_rs::common::VideoID<'a>>,
    token: String,
}

impl PostQuery for WatchContinuation<'_> {
    fn header(&self) -> serde_json::Map<String, Value> {
        self.base.header()
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![
            ("ctoken", self.token.as_str().into()),
            ("continuation", self.token.as_str().into()),
        ]
    }
    fn path(&self) -> &str {
        self.base.path()
    }
}

impl ParseFrom<WatchContinuation<'_>> for RawPage {
    fn parse_from(_: ProcessedResult<WatchContinuation<'_>>) -> ytmapi_rs::Result<Self> {
        Ok(RawPage)
    }
}

impl<A: ytmapi_rs::auth::AuthToken> Query<A> for WatchContinuation<'_> {
    type Output = RawPage;
    type Method = PostMethod;
}
```

(If the `VideoID` import makes the fully-qualified path redundant, use the plain `VideoID<'a>` — match however `PlaylistID` is referenced in the file.)

- [ ] **Step 4: Compile-check tests and run the crate suite**

Run: `cargo test -p hm-ytmusic 2>&1 | tail -3`
Expected: all existing tests + Task 1's 8 still pass; the live test is ignored.

- [ ] **Step 5: Run the live test if a session exists (best effort)**

Run: `cargo test -p hm-ytmusic --  --ignored live_radio 2>&1 | tail -6`
Expected: PASS with `radio page 1: N tracks` / `radio page 2: M tracks` printed, or the visible `skipping: no session…` line. A hard failure here is a real bug — stop and investigate, don't proceed.

- [ ] **Step 6: Commit**

```bash
git add crates/hm-ytmusic/src/lib.rs
git commit -m "feat(ytmusic): radio + continuation fetch through the next endpoint"
```

---

### Task 3: Tauri commands + IPC wrappers

**Files:**
- Modify: `src-tauri/src/commands/ytmusic.rs` (after `ytmusic_artist_page`, ~line 371)
- Modify: `src-tauri/src/lib.rs` (register next to `commands::ytmusic::ytmusic_search`, ~line 683)
- Modify: `src/lib/types.ts` (after `YtTrack`, ~line 490)
- Modify: `src/lib/ipc.ts` (after `ytmusicSearchSuggestions`, ~line 375)

**Interfaces:**
- Consumes: `YtMusicState::{radio, radio_continue}` (Task 2). Add `RadioBatch` to the `hm_ytmusic` import at `commands/ytmusic.rs:24`.
- Produces: TS `ytmusicRadio(videoId: string): Promise<RadioBatch>` and `ytmusicRadioContinue(videoId: string, token: string): Promise<RadioBatch>`; TS `interface RadioBatch { tracks: YtTrack[]; continuation: string | null }` — used by Task 6.

- [ ] **Step 1: Add the commands**

```rust
/// The endless "up next" YT Music derives from one song — its radio. Returns
/// the first page (~25–50 similar tracks) and the token for the next one.
#[tauri::command]
pub async fn ytmusic_radio(
    state: State<'_, YtMusicState>,
    video_id: String,
) -> Result<RadioBatch, IpcError> {
    state
        .radio(&video_id)
        .await
        .map_err(|e| IpcError::new("ytmusic", e))
}

/// The next page of a radio. `video_id` is the seed the radio was started
/// from — the wire format re-POSTs the full body plus the token.
#[tauri::command]
pub async fn ytmusic_radio_continue(
    state: State<'_, YtMusicState>,
    video_id: String,
    token: String,
) -> Result<RadioBatch, IpcError> {
    state
        .radio_continue(&video_id, &token)
        .await
        .map_err(|e| IpcError::new("ytmusic", e))
}
```

Register both in `src-tauri/src/lib.rs` beside the other ytmusic commands:

```rust
            commands::ytmusic::ytmusic_radio,
            commands::ytmusic::ytmusic_radio_continue,
```

- [ ] **Step 2: Add the TS type and wrappers**

`src/lib/types.ts` (after `YtTrack`):

```ts
/** One page of a song radio: similar tracks plus the token for the next page.
 *  A missing token is rare — radio panels normally chain forever. */
export interface RadioBatch {
  tracks: YtTrack[];
  continuation: string | null;
}
```

`src/lib/ipc.ts` (import `RadioBatch` in the existing `@/lib/types` type import; add after `ytmusicSearchSuggestions`):

```ts
export function ytmusicRadio(videoId: string): Promise<RadioBatch> {
  return invoke<RadioBatch>("ytmusic_radio", { videoId });
}

export function ytmusicRadioContinue(videoId: string, token: string): Promise<RadioBatch> {
  return invoke<RadioBatch>("ytmusic_radio_continue", { videoId, token });
}
```

- [ ] **Step 3: Verify both sides compile**

Run: `cargo check -p hypemuzik 2>&1 | tail -3 && pnpm exec tsc --noEmit 2>&1 | tail -3`
Expected: both clean. (`hypemuzik` is the src-tauri package name; if `cargo check -p hypemuzik` errors with "package not found", use `cargo check --workspace`.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/ytmusic.rs src-tauri/src/lib.rs src/lib/types.ts src/lib/ipc.ts
git commit -m "feat(ytmusic): radio IPC commands and wrappers"
```

---

### Task 4: Persisted `autoplay` flag on PlaybackState

**Files:**
- Modify: `crates/hm-core/src/types.rs` (`PlaybackState` ~line 320, its `Default` ~line 330)
- Modify: `crates/hm-audio/src/engine.rs` (`set_playback` ~line 932, new `set_autoplay` after `set_data_saver` ~line 946)
- Modify: `src-tauri/src/commands/engine.rs` (after `engine_set_data_saver`, ~line 253)
- Modify: `src-tauri/src/lib.rs` (register `engine_set_autoplay` next to `commands::engine::engine_set_data_saver`)
- Modify: `src/lib/types.ts` (`PlaybackState` ~line 150)
- Modify: `src/lib/ipc.ts` (next to the existing `engineSetDataSaver` wrapper)
- Modify: `src/stores/engine.ts` (default state ~line 166, interface ~line 366, action next to `setDataSaver` ~line 944)

**Interfaces:**
- Produces: Rust `PlaybackState.autoplay: bool` (serde default `true`), `AudioEngine::set_autoplay(on: bool)`, command `engine_set_autoplay`; TS `PlaybackState.autoplay: boolean`, `engineSetAutoplay(on: boolean): Promise<void>`, store action `setAutoplay: (on: boolean) => void` — used by Tasks 6 and 8.

- [ ] **Step 1: Rust struct + default**

`crates/hm-core/src/types.rs`:

```rust
pub struct PlaybackState {
    /// Play a track list with no silence between tracks.
    pub gapless: bool,
    /// Crossfade duration in seconds (0 = off). Implies gapless when > 0.
    pub crossfade_secs: f32,
    /// Low-bandwidth mode: stream progressively (no full-download / prefetch),
    /// bigger buffers. Forces progressive single-track playback for cloud/phone.
    pub data_saver: bool,
    /// Keep the music going: a YT Music queue that runs out extends itself
    /// with the song radio of its last track. Read by the frontend only;
    /// stored here so the choice survives restarts. Defaults on — saved
    /// states from before the field existed get the new behaviour.
    #[serde(default = "default_autoplay")]
    pub autoplay: bool,
}

fn default_autoplay() -> bool {
    true
}
```

In `impl Default for PlaybackState`, add `autoplay: true,`.

- [ ] **Step 2: Engine setter + preserve in `set_playback`**

`crates/hm-audio/src/engine.rs` — `set_playback` builds the struct literally; keep the flag it doesn't own:

```rust
        self.update(|s| {
            s.playback = hm_core::PlaybackState { gapless, crossfade_secs: crossfade, data_saver: s.playback.data_saver, autoplay: s.playback.autoplay };
        });
```

After `set_data_saver`:

```rust
    /// Toggle Autoplay (endless queue extension). The engine never reads this —
    /// the frontend queue does — it lives here to ride the state autosave.
    pub fn set_autoplay(&self, on: bool) {
        self.update(|s| s.playback.autoplay = on);
    }
```

- [ ] **Step 3: Command + registration**

`src-tauri/src/commands/engine.rs`:

```rust
/// Toggle Autoplay — whether a finished YT Music queue extends itself with radio.
#[tauri::command]
pub fn engine_set_autoplay(engine: State<'_, AudioEngine>, on: bool) {
    engine.set_autoplay(on);
}
```

Register `commands::engine::engine_set_autoplay,` in `src-tauri/src/lib.rs`.

- [ ] **Step 4: TS mirror**

`src/lib/types.ts` `PlaybackState`:

```ts
export interface PlaybackState {
  gapless: boolean;
  /** Crossfade duration in seconds (0 = off; implies gapless when > 0). */
  crossfadeSecs: number;
  /** Low-bandwidth mode: progressive streaming, no prefetch, bigger buffers. */
  dataSaver: boolean;
  /** Keep the music going: a finished YT Music queue extends itself with the
   *  song radio of its last track. */
  autoplay: boolean;
}
```

`src/lib/ipc.ts` (next to `engineSetDataSaver`):

```ts
export function engineSetAutoplay(on: boolean): Promise<void> {
  return invoke<void>("engine_set_autoplay", { on });
}
```

`src/stores/engine.ts`:
- default state line 166 → `playback: { gapless: true, crossfadeSecs: 0, dataSaver: false, autoplay: true },`
- interface (after `setDataSaver`'s entry near line 366): `setAutoplay: (on: boolean) => void;`
- action (after `setDataSaver`, ~line 944):

```ts
    setAutoplay: (on) => {
      set((s) => ({ state: { ...s.state, playback: { ...s.state.playback, autoplay: on } } }));
      void engineSetAutoplay(on).catch(() => {});
    },
```

(Add `engineSetAutoplay` to the big `@/lib/ipc` import at the top.)

- [ ] **Step 5: Verify**

Run: `cargo check --workspace 2>&1 | tail -3 && pnpm exec tsc --noEmit 2>&1 | tail -3 && pnpm test -- --run src/stores/engine.test.ts 2>&1 | tail -4`
Expected: all clean; existing engine tests unaffected.

- [ ] **Step 6: Commit**

```bash
git add crates/hm-core/src/types.rs crates/hm-audio/src/engine.rs src-tauri/src/commands/engine.rs src-tauri/src/lib.rs src/lib/types.ts src/lib/ipc.ts src/stores/engine.ts
git commit -m "feat(player): persisted Autoplay flag on playback settings"
```

---

### Task 5: Pure radio-session logic (`src/stores/radio.ts`)

**Files:**
- Create: `src/stores/radio.ts`
- Create: `src/stores/radio.test.ts`

**Interfaces:**
- Consumes: `import type { QueueItem, RepeatMode } from "@/stores/engine"` (type-only — no runtime cycle), `import type { YtTrack } from "@/lib/types"`.
- Produces (used by Task 6): `RadioSession`, `RADIO_LOW_WATER = 5`, `radioStep(args): RadioStep`, `dedupeRadioTracks(queue, incoming): YtTrack[]`.

- [ ] **Step 1: Write the failing tests**

`src/stores/radio.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { dedupeRadioTracks, radioStep, RADIO_LOW_WATER } from "@/stores/radio";
import { ytmusicItem } from "@/stores/engine";
import type { YtTrack } from "@/lib/types";

const track = (videoId: string, over: Partial<YtTrack> = {}): YtTrack => ({
  videoId,
  title: `Song ${videoId}`,
  artist: "Artist",
  album: null,
  durationSecs: 200,
  thumbnail: null,
  playlistId: "RDAMVMseed",
  playlistTitle: "Radio",
  isAvailable: true,
  hasVideo: false,
  ...over,
});

/** A store snapshot near the end of a 10-track all-YT queue. */
const base = () => ({
  autoplay: true,
  fetching: false,
  session: { seedId: "seed", continuation: "tok" } as {
    seedId: string;
    continuation: string | null;
  } | null,
  orderLen: 10,
  orderPos: 10 - RADIO_LOW_WATER - 1, // exactly LOW_WATER tracks remain ahead
  allYtMusic: true,
  lastVideoId: "last",
  repeat: "off" as const,
});

describe("radioStep", () => {
  it("continues the session with its token when the queue runs low", () => {
    expect(radioStep(base())).toEqual({ kind: "continue", seedId: "seed", token: "tok" });
  });

  it("does nothing while plenty of queue remains", () => {
    expect(radioStep({ ...base(), orderPos: 0 })).toBeNull();
  });

  it("is fully gated by the Autoplay switch", () => {
    expect(radioStep({ ...base(), autoplay: false })).toBeNull();
  });

  it("never stacks fetches", () => {
    expect(radioStep({ ...base(), fetching: true })).toBeNull();
  });

  it("lets repeat win — a loop over a growing queue would never come round", () => {
    expect(radioStep({ ...base(), repeat: "all" })).toBeNull();
    expect(radioStep({ ...base(), repeat: "one" })).toBeNull();
  });

  it("re-seeds from the last track when the chain has no token", () => {
    expect(radioStep({ ...base(), session: { seedId: "seed", continuation: null } })).toEqual({
      kind: "reseed",
      seedId: "last",
    });
  });

  it("starts a session for a finishing all-YT queue with none", () => {
    expect(radioStep({ ...base(), session: null })).toEqual({ kind: "start", seedId: "last" });
  });

  it("never grows a local/phone/cloud queue", () => {
    expect(radioStep({ ...base(), session: null, allYtMusic: false })).toBeNull();
  });

  it("does nothing on an empty or unstarted queue", () => {
    expect(radioStep({ ...base(), orderLen: 0, orderPos: -1 })).toBeNull();
  });
});

describe("dedupeRadioTracks", () => {
  const queue = [ytmusicItem(track("a")), ytmusicItem(track("b"))];

  it("drops tracks already queued — continuation pages overlap", () => {
    const out = dedupeRadioTracks(queue, [track("b"), track("c")]);
    expect(out.map((t) => t.videoId)).toEqual(["c"]);
  });

  it("drops in-batch duplicates but keeps the order", () => {
    const out = dedupeRadioTracks(queue, [track("d"), track("c"), track("d")]);
    expect(out.map((t) => t.videoId)).toEqual(["d", "c"]);
  });

  it("drops unavailable tracks — they can't stream", () => {
    const out = dedupeRadioTracks(queue, [track("e", { isAvailable: false })]);
    expect(out).toEqual([]);
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `pnpm test -- --run src/stores/radio.test.ts 2>&1 | tail -4`
Expected: FAIL — module `@/stores/radio` not found.

- [ ] **Step 3: Implement**

`src/stores/radio.ts`:

```ts
import type { QueueItem, RepeatMode } from "@/stores/engine";
import type { YtTrack } from "@/lib/types";

/**
 * The decision logic behind the endless queue, kept pure so it's testable.
 * The engine store owns the session and the fetches; this answers one
 * question: given where playback is, what — if anything — should radio do?
 */

/** A live radio session: the seed the queue grew from and where the next page
 *  continues. A `null` continuation means the chain broke — the next step
 *  re-seeds from the end of the queue instead of stopping. */
export interface RadioSession {
  seedId: string;
  continuation: string | null;
}

/** Fetch more once this few unplayed tracks remain ahead of the listener. */
export const RADIO_LOW_WATER = 5;

export type RadioStep =
  | { kind: "continue"; seedId: string; token: string }
  | { kind: "reseed"; seedId: string }
  | { kind: "start"; seedId: string }
  | null;

/** What radio should do now that playback sits at `orderPos`. */
export function radioStep(args: {
  autoplay: boolean;
  fetching: boolean;
  session: RadioSession | null;
  orderLen: number;
  orderPos: number;
  /** Whole queue is YT Music tracks — radio can only grow those. */
  allYtMusic: boolean;
  /** videoId of the last track in play order: the seed for extension. */
  lastVideoId: string | null;
  repeat: RepeatMode;
}): RadioStep {
  const { autoplay, fetching, session, orderLen, orderPos } = args;
  if (!autoplay || fetching || orderLen === 0 || orderPos < 0) return null;
  // Repeat loops the current list; against an ever-growing queue the loop
  // would never come round, so repeat wins over radio.
  if (args.repeat !== "off") return null;
  const remaining = orderLen - orderPos - 1;
  if (remaining > RADIO_LOW_WATER) return null;
  if (session) {
    if (session.continuation) {
      return { kind: "continue", seedId: session.seedId, token: session.continuation };
    }
    return args.lastVideoId ? { kind: "reseed", seedId: args.lastVideoId } : null;
  }
  if (!args.allYtMusic || !args.lastVideoId) return null;
  return { kind: "start", seedId: args.lastVideoId };
}

/** Incoming radio tracks not already in the queue (continuation pages
 *  overlap) and actually streamable, original order kept. */
export function dedupeRadioTracks(queue: QueueItem[], incoming: YtTrack[]): YtTrack[] {
  const seen = new Set(queue.map((q) => q.id));
  const out: YtTrack[] = [];
  for (const t of incoming) {
    if (!t.isAvailable || seen.has(t.videoId)) continue;
    seen.add(t.videoId);
    out.push(t);
  }
  return out;
}
```

- [ ] **Step 4: Run to verify pass**

Run: `pnpm test -- --run src/stores/radio.test.ts 2>&1 | tail -4`
Expected: 12 passed.

- [ ] **Step 5: Commit**

```bash
git add src/stores/radio.ts src/stores/radio.test.ts
git commit -m "feat(player): pure radio-session decision logic"
```

---

### Task 6: Engine store wiring (session, append, replenish, seams)

**Files:**
- Modify: `src/stores/engine.ts`
- Modify: `src/stores/engine.test.ts` (radioItem test)

**Interfaces:**
- Consumes: `radioStep`, `dedupeRadioTracks`, `RadioSession` (Task 5); `ytmusicRadio`, `ytmusicRadioContinue` (Task 3); `state.playback.autoplay` (Task 4).
- Produces (used by Tasks 7–8): `QueueItem.autoAdded?: boolean`; exported `radioItem(t: YtTrack): QueueItem`; store action `playYtRadio: (seed: YtTrack) => void`.

All edits are in `src/stores/engine.ts` unless noted.

- [ ] **Step 1: Failing test for `radioItem`**

Append to `src/stores/engine.test.ts` (reuse its existing `track()` helper):

```ts
describe("radioItem", () => {
  it("is a ytmusic queue item marked auto-added — the queue UI badges it", () => {
    expect(radioItem(track())).toEqual({ ...ytmusicItem(track()), autoAdded: true });
  });
});
```

and add `radioItem` to the import from `@/stores/engine`.

Run: `pnpm test -- --run src/stores/engine.test.ts 2>&1 | tail -4` → FAIL (no export `radioItem`).

- [ ] **Step 2: Type + helper**

In the `QueueItem` interface (after `radioUrl?: string;` ~line 110):

```ts
  /** Queued by radio (Autoplay), not by the user — the queue UI badges these. */
  autoAdded?: boolean;
```

After `ytmusicItem` (~line 230):

```ts
/** A radio pick: the same ytmusic item, marked auto-added. */
export function radioItem(t: YtTrack): QueueItem {
  return { ...ytmusicItem(t), autoAdded: true };
}
```

Run: `pnpm test -- --run src/stores/engine.test.ts 2>&1 | tail -4` → PASS.

- [ ] **Step 3: Imports + module-level session state**

Add to imports: `ytmusicRadio, ytmusicRadioContinue` (in the `@/lib/ipc` block) and:

```ts
import { dedupeRadioTracks, radioStep, type RadioSession } from "@/stores/radio";
```

Next to `let gaplessQueueRunning = false;` (~line 421):

```ts
  // The endless-queue (radio) session. Module-level like gaplessQueueRunning:
  // the UI never reads it — it reads `autoAdded` off the items and the
  // Autoplay flag off playback settings.
  let radioSession: RadioSession | null = null;
  let radioFetching = false;
  // Bumped whenever the queue is replaced, so a radio page that lands after
  // the user has moved on can't graft one radio onto another queue.
  let radioEpoch = 0;
  // How many order positions the engine's gapless queue was handed — appended
  // radio tracks beyond this are invisible to it (see advanceOnEnd).
  let gaplessQueueLen = 0;
  // The queue finished on its own (nothing left), as opposed to the user
  // stopping it. Only then may a late-arriving radio batch resume playback.
  let endedNaturally = false;
```

- [ ] **Step 4: Append + fetch/replenish helpers**

After `setQueueAndPlay` (~line 725), add:

```ts
  /** Extend the live queue in place: the current stream keeps running; the
   *  new items only lengthen `queue` and `order`. Appended in listed order
   *  even under shuffle — a radio is already a curated order. */
  const appendQueueItems = (items: QueueItem[]) => {
    if (items.length === 0) return;
    const { queue, order } = get();
    const base = queue.length;
    set({
      queue: [...queue, ...items],
      order: [...order, ...items.map((_, i) => base + i)],
    });
  };

  /** If the queue ran dry before a radio page landed, pick up where it ended. */
  const resumeIfEndedNaturally = () => {
    if (!endedNaturally) return;
    const { order, orderPos } = get();
    if (orderPos < order.length - 1) {
      endedNaturally = false;
      startPlayback(orderPos + 1);
    }
  };

  /** Run one radio fetch and append what it returns. Failures are silent —
   *  the next track-advance retries via maybeExtendRadio; playback is never
   *  interrupted for a queue that only might run out. */
  const fetchRadio = (step: NonNullable<ReturnType<typeof radioStep>>) => {
    radioFetching = true;
    const epoch = radioEpoch;
    const req =
      step.kind === "continue"
        ? ytmusicRadioContinue(step.seedId, step.token)
        : ytmusicRadio(step.seedId);
    void req
      .then((batch) => {
        if (epoch !== radioEpoch) return; // the queue moved on mid-flight
        radioSession = { seedId: step.seedId, continuation: batch.continuation };
        const fresh = dedupeRadioTracks(get().queue, batch.tracks);
        appendQueueItems(fresh.map(radioItem));
        resumeIfEndedNaturally();
      })
      .catch((e) => console.warn("radio fetch failed:", e))
      .finally(() => {
        radioFetching = false;
      });
  };

  /** Keep an endless queue endless — called on every track advance. */
  const maybeExtendRadio = () => {
    const { state, queue, order, orderPos, repeat } = get();
    const lastQi = order.length > 0 ? order[order.length - 1]! : -1;
    const last = lastQi >= 0 ? queue[lastQi] : undefined;
    const step = radioStep({
      autoplay: state.playback.autoplay,
      fetching: radioFetching,
      session: radioSession,
      orderLen: order.length,
      orderPos,
      allYtMusic: order.length > 0 && order.every((i) => queue[i]?.ytTrack != null),
      lastVideoId: last?.ytTrack?.videoId ?? null,
      repeat,
    });
    if (step) fetchRadio(step);
  };
```

- [ ] **Step 5: Hook the lifecycle points**

1. **`setQueueAndPlay`** (top of the function body, before anything else):

```ts
    // A new queue is a new listening intent: the old radio session is over.
    radioSession = null;
    radioEpoch += 1;
    endedNaturally = false;
```

2. **`startPlayback`** — right after `gaplessQueueRunning = ...` is assigned (~line 510):

```ts
    gaplessQueueLen = gaplessQueueRunning ? order.length : 0;
```

and at the very end of `startPlayback` (after `fillNowPlayingCover(item);`):

```ts
    endedNaturally = false;
    maybeExtendRadio();
```

3. **`applyQueueIndex`** — at the end of its body (after `fillNowPlayingCover(item);`):

```ts
      maybeExtendRadio();
```

4. **`advanceOnEnd`** — replace the `gaplessQueueRunning` branch:

```ts
    if (gaplessQueueRunning) {
      // Radio appended past the list the engine was handed — the engine never
      // knew those tracks. Continue at the first one.
      if (order.length > gaplessQueueLen) {
        startPlayback(gaplessQueueLen);
        return;
      }
      // The whole gapless list just finished.
      if (repeat === "all" && order.length > 0) startPlayback(0);
      else {
        endedNaturally = true;
        set(idleState());
      }
      return;
    }
```

and in the single-track fall-through at the bottom of `advanceOnEnd`:

```ts
    const np = stepOrder(orderPos, order.length, repeat, 1);
    if (np !== null) startPlayback(np);
    else {
      endedNaturally = true;
      set(idleState());
    }
```

5. **`stop`** — find the `stop:` action and add `endedNaturally = false;` as its first line (an explicit stop must never be resumed by a late batch).

- [ ] **Step 6: The `playYtRadio` action**

Interface (after `playQueueItems` ~line 386):

```ts
  /** Play `seed` now and grow the queue behind it with YT Music's song radio
   *  — the endless "similar tracks" queue. */
  playYtRadio: (seed: YtTrack) => void;
```

(Ensure `YtTrack` is in the type imports from `@/lib/types` — it already is, via `ytmusicItem`.)

Implementation (after the `playQueueItems` line ~1092):

```ts
    playYtRadio: (seed) => {
      // The seed plays immediately; the similar tracks stream in behind it.
      setQueueAndPlay([ytmusicItem(seed)], 0); // also tears down any old session
      radioSession = { seedId: seed.videoId, continuation: null };
      fetchRadio({ kind: "start", seedId: seed.videoId });
    },
```

- [ ] **Step 7: Verify**

Run: `pnpm exec tsc --noEmit 2>&1 | tail -3 && pnpm test -- --run 2>&1 | tail -4`
Expected: typecheck clean; full vitest suite green.

- [ ] **Step 8: Commit**

```bash
git add src/stores/engine.ts src/stores/engine.test.ts
git commit -m "feat(player): endless radio session in the queue store"
```

---

### Task 7: Search / shelf trigger (`explore.ts`)

**Files:**
- Modify: `src/stores/explore.ts` (the `open` handler ~line 258; delete `searchQueue` ~lines 130–170 and its `ytmusicItem` import if now unused)
- Modify: `src/stores/explore.test.ts` (delete the `searchQueue` describe block)

**Interfaces:**
- Consumes: `playYtRadio` (Task 6).

- [ ] **Step 1: Replace the song/video click path**

In `open`, replace the whole `if (item.kind === "song" || item.kind === "video") { ... }` block with:

```ts
      if (item.kind === "song" || item.kind === "video") {
        const playable = tracks.filter((t) => t.isAvailable);
        if (playable.length === 0) {
          set({ openError: `"${item.title}" can't be played.` });
          return;
        }
        // One click = this song plus its radio: the queue behind it fills with
        // YT Music's own similar-track picks (engine.playYtRadio), exactly as
        // the YT Music client does. This replaces the old "queue the rest of
        // the search page" behaviour: search results are what matched the
        // words; the radio is what matches the taste.
        useEngineStore.getState().playYtRadio(playable[0]!);
        return;
      }
```

- [ ] **Step 2: Delete `searchQueue` and its tests**

Remove the `searchQueue` function and its doc comment from `explore.ts`; remove its import/usages (`ytmusicItem` import too if nothing else uses it). Remove the `searchQueue` describe block and import from `explore.test.ts`.

- [ ] **Step 3: Verify**

Run: `pnpm exec tsc --noEmit 2>&1 | tail -3 && pnpm test -- --run src/stores/explore.test.ts 2>&1 | tail -4`
Expected: clean; remaining explore tests green.

- [ ] **Step 4: Commit**

```bash
git add src/stores/explore.ts src/stores/explore.test.ts
git commit -m "feat(explore): a played search result starts its radio"
```

---

### Task 8: Queue UI — Autoplay switch + radio badge

**Files:**
- Modify: `src/features/player/QueueList.tsx`

**Interfaces:**
- Consumes: `Switch` from `@/components/Switch` (`{ checked, onChange, label }`), `s.state.playback.autoplay` + `s.setAutoplay` (Task 4), `item.autoAdded` (Task 6).

**Design note:** the spec's "thin divider where auto-added tracks begin" collides with `VirtualList`'s uniform row height; a per-row badge marks the same boundary without breaking virtualization. (Approved deviation — visual only.)

- [ ] **Step 1: Autoplay switch in the "Next up" header**

In `QueueList`, add selectors:

```ts
  const autoplay = useEngineStore((s) => s.state.playback.autoplay);
  const setAutoplay = useEngineStore((s) => s.setAutoplay);
```

Replace the `Next up` `<h3>` with:

```tsx
      <div className="flex shrink-0 items-center justify-between px-3 pb-2 pt-4">
        <h3 className="text-base font-bold tracking-tight">Next up</h3>
        <label className="flex items-center gap-2 text-[13px] text-text-muted">
          Autoplay
          <Switch checked={autoplay} onChange={setAutoplay} label="Autoplay — keep similar tracks coming" />
        </label>
      </div>
```

Add `import { Switch } from "@/components/Switch";`.

- [ ] **Step 2: Radio badge on auto-added rows**

In `QueueRow`, replace the artist `<p>` with:

```tsx
        <p className="mt-1 flex items-center gap-1.5 truncate text-[13px] text-text-muted">
          {item.autoAdded && (
            <span className="shrink-0 rounded-full border border-border-strong px-1.5 text-[10px] font-semibold uppercase tracking-wide text-text-faint">
              Radio
            </span>
          )}
          <span className="truncate">{item.artist ?? "Unknown artist"}</span>
        </p>
```

- [ ] **Step 3: Verify**

Run: `pnpm exec tsc --noEmit 2>&1 | tail -3 && pnpm build 2>&1 | tail -3`
Expected: both clean.

- [ ] **Step 4: Commit**

```bash
git add src/features/player/QueueList.tsx
git commit -m "feat(queue): Autoplay switch and radio badge in Up Next"
```

---

### Task 9: Full verification, push, memory

- [ ] **Step 1: Whole-workspace verification**

```bash
cargo clippy --workspace --all-targets 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
pnpm exec tsc --noEmit 2>&1 | tail -3
pnpm test -- --run 2>&1 | tail -4
pnpm build 2>&1 | tail -3
```

Expected: zero clippy warnings in touched crates, all tests green, build clean. Known pre-existing flake: an `hm-remote` iroh timing test can fail under CPU contention — rerun it alone before treating it as a regression.

- [ ] **Step 2: Live radio test (best effort)**

Run: `cargo test -p hm-ytmusic -- --ignored live_radio 2>&1 | tail -6`
Expected: PASS with page sizes printed, or the visible keychain skip.

- [ ] **Step 3: Push**

```bash
git push -u origin feat/radio-autoqueue
```

- [ ] **Step 4: Update memory**

Update `~/.claude/projects/-Users-bruno-me-COTE/memory/hypemuzik_desktop_ytmusic_radio.md`: implemented on branch `feat/radio-autoqueue`; note the engine-queue seam (`gaplessQueueLen`) and the `radioEpoch` staleness guard; flag NOT GUI-tested (dev builds hang on splash — verify via `--ignored` live tests).
