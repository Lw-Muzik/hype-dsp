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
                if let Some(t) = v
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
