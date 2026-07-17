//! Playlist track rows, read straight from the API's JSON.
//!
//! # Why this module owns a parser
//!
//! `get_playlist_tracks` requires a **channel id** on every video row:
//!
//! ```text
//! let channel_id = data
//!     .borrow_pointer(flex_column_item_pointer(1))?
//!     .take_value_pointer(concatcp!(TEXT_RUN, NAVIGATION_BROWSE_ID))?;
//! ```
//!
//! and `parse_playlist_items` collects with `Result<_>`, so one row without it
//! destroys the whole page. Measured on a real library: `"Chosen (feat. Ty Dolla
//! $ign & Tyga)"` is credited to *"Tyga & Blxst"* — a collaboration with no
//! single channel to link — and that one row at index 93 hid all 134 tracks of
//! the playlist containing it. Four playlists died this way; a fifth stopped at
//! 300 of 389 when a continuation page hit the same thing.
//!
//! [`YtTrack`] has no channel-id field, so we never needed that value: upstream
//! demanded it for its own struct. Reading the rows ourselves drops the
//! requirement entirely rather than fabricating a value to satisfy it.
//!
//! Walking for `musicResponsiveListItemRenderer` also makes this shape-agnostic:
//! the first page nests rows under `twoColumnBrowseResultsRenderer` and
//! continuation pages under `continuationItems`, and neither matters if you just
//! look for the rows.
//!
//! **Nothing here may hard-fail** — an unreadable row is skipped, never fatal.
//! That is the whole point; see [`crate::explore`] for the same rule.

use crate::nav::{best_thumbnail, join_runs};
use crate::{parse_duration, YtPlaylist, YtTrack};
use serde_json::Value;

/// Rows YouTube marks as unplayable are listed but not playable.
const GREY_OUT: &str = "MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT";

/// Library uploads have no resolvable video id, and podcast episodes aren't what
/// this library is for. Everything else (songs, and the various video flavours)
/// is music. Mirrors the old `map_item` match, which upstream's typed enum used
/// to make for us.
/// Where a row states what kind of thing it is.
///
/// Assembled from the same three parts upstream concatenates (`PLAY_BUTTON` +
/// `/playNavigationEndpoint` + `NAVIGATION_VIDEO_TYPE`) rather than written out
/// as one literal. The literal that was here had **`/watchEndpoint` missing** —
/// the first segment of upstream's third constant, lost when the pieces were
/// inlined by hand. Measured against a real response: the old path matched
/// 0/100 rows, this one matches 98/100. Keeping the seams visible is what makes
/// a dropped segment legible instead of invisible.
const VIDEO_TYPE_PATH: &str = concat!(
    "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer",
    "/playNavigationEndpoint",
    "/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType",
);

fn is_music(video_type: Option<&str>) -> bool {
    !matches!(
        video_type,
        Some("MUSIC_VIDEO_TYPE_PODCAST_EPISODE") | Some("MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK")
    )
}

/// Whether this row has footage, as opposed to a song rendered as a still.
///
/// `ATV` is YouTube's marker for an audio entity. It still has "video"
/// renditions, but they're a square 1080×1080 still at ~95 kbps — the cover art
/// again. Everything else that reaches here (OMV, UGC, official-source, …) is
/// real video.
///
/// `None` means the row didn't carry a type, which is *not* the same as being a
/// song — so it earns `false` (offer no Video tab we might not be able to fill)
/// while [`is_music`] independently answers `true` (keep the row). Two questions
/// with two different safe answers is the reason this is an `Option` and not a
/// defaulted `&str`: collapsing them behind one `unwrap_or("…ATV")` is what hid
/// a broken pointer for an entire release — every row reported "song, no video",
/// and neither the tab nor the podcast filter ever fired.
fn has_video(video_type: Option<&str>) -> bool {
    matches!(video_type, Some(t) if t != "MUSIC_VIDEO_TYPE_ATV")
}

/// Every playable row on one page of a playlist response.
pub fn parse_page(json: &Value, playlist: &YtPlaylist) -> Vec<YtTrack> {
    let mut rows = Vec::new();
    collect_rows(json, &mut rows);
    rows.iter()
        .filter_map(|row| parse_row(row, playlist))
        .collect()
}

/// The token for the next page, if this one has a successor.
///
/// Extracted here rather than taken from upstream for the same reason the rows
/// are: `raw_json_stream` sources its token from the very parser that dies on a
/// channel-less row, so the playlists that need paging most are exactly the ones
/// it stops paging. It lives on a trailing `continuationItemRenderer` sibling of
/// the rows.
pub fn next_page_token(json: &Value) -> Option<String> {
    fn walk(v: &Value) -> Option<String> {
        match v {
            Value::Object(o) => {
                if let Some(t) = o
                    .get("continuationItemRenderer")
                    .and_then(|c| c.pointer("/continuationEndpoint/continuationCommand/token"))
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

fn collect_rows<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(o) => {
            if let Some(row) = o.get("musicResponsiveListItemRenderer") {
                out.push(row);
                return; // rows don't nest
            }
            for val in o.values() {
                collect_rows(val, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|i| collect_rows(i, out)),
        _ => {}
    }
}

/// The text of one flexible column ("/flexColumns/N"), runs joined.
fn flex_text(row: &Value, col: usize) -> Option<String> {
    join_runs(row.pointer(&format!(
        "/flexColumns/{col}/musicResponsiveListItemFlexColumnRenderer/text/runs"
    )))
}

fn parse_row(row: &Value, playlist: &YtPlaylist) -> Option<YtTrack> {
    // No video id → nothing to stream, whatever else the row says.
    let video_id = row
        .pointer("/playlistItemData/videoId")
        .and_then(Value::as_str)?;

    let title = flex_text(row, 0)?;
    // A removed track keeps its row but not its identity.
    if title == "Song deleted" {
        return None;
    }

    let video_type = row.pointer(VIDEO_TYPE_PATH).and_then(Value::as_str);
    if !is_music(video_type) {
        return None;
    }

    let is_available = row
        .pointer("/musicItemRendererDisplayPolicy")
        .and_then(Value::as_str)
        .map(|p| p != GREY_OUT)
        .unwrap_or(true);

    Some(YtTrack {
        video_id: video_id.to_string(),
        title,
        // Column 1 is the artist/uploader; column 2 the album, which playlists
        // often leave blank.
        artist: flex_text(row, 1),
        album: flex_text(row, 2),
        duration_secs: duration_of(row).as_deref().and_then(parse_duration),
        thumbnail: best_thumbnail(
            row.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"),
        ),
        playlist_id: playlist.id.clone(),
        playlist_title: playlist.title.clone(),
        is_available,
        has_video: has_video(video_type),
    })
}

/// Duration lives in the first fixed column, as runs or a bare string depending
/// on the row.
fn duration_of(row: &Value) -> Option<String> {
    let text = row.pointer("/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text")?;
    join_runs(text.pointer("/runs")).or_else(|| {
        text.pointer("/simpleText")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn playlist() -> YtPlaylist {
        YtPlaylist {
            id: "VLPL1".into(),
            title: "archive".into(),
            author: "Bruno".into(),
            track_count: None,
            thumbnail: None,
        }
    }

    /// Builds a row. `artist_browse` mirrors the field upstream demands: `None`
    /// is the collaboration-credit case that used to kill the whole playlist.
    fn row(title: &str, video_id: &str, artist: &str, artist_browse: Option<&str>) -> Value {
        let mut artist_run = json!({ "text": artist });
        if let Some(id) = artist_browse {
            artist_run["navigationEndpoint"] = json!({ "browseEndpoint": { "browseId": id } });
        }
        json!({
            "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": video_id },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": title }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [artist_run] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "Some Album" }] } } }
                ],
                "fixedColumns": [
                    { "musicResponsiveListItemFixedColumnRenderer": {
                        "text": { "runs": [{ "text": "3:01" }] } } }
                ],
                "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                    { "url": "small.jpg", "width": 60, "height": 60 },
                    { "url": "big.jpg", "width": 544, "height": 544 }
                ]}}}
            }
        })
    }

    /// First-page shape: rows buried under the two-column container.
    fn first_page(rows: Vec<Value>) -> Value {
        json!({ "contents": { "twoColumnBrowseResultsRenderer": { "secondaryContents": {
            "sectionListRenderer": { "contents": [{ "musicPlaylistShelfRenderer": {
                "contents": rows } }] } } } } })
    }

    /// Continuation shape: a different container entirely.
    fn continuation_page(rows: Vec<Value>) -> Value {
        json!({ "onResponseReceivedActions": [{ "appendContinuationItemsAction": {
            "continuationItems": rows } }] })
    }

    /// The bug this module exists for: one collaboration credit with no channel
    /// link must cost that row's *artist link*, not the entire playlist.
    #[test]
    fn a_row_without_an_artist_browse_id_is_kept() {
        let json = first_page(vec![
            row("yawa", "v1", "Fireboy DML", Some("UC1")),
            row("Chosen", "v93", "Tyga & Blxst", None), // no channel — used to be fatal
            row("Gyal", "v2", "Charly Black", Some("UC2")),
        ]);
        let tracks = parse_page(&json, &playlist());
        assert_eq!(
            tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["yawa", "Chosen", "Gyal"]
        );
        assert_eq!(tracks[1].artist.as_deref(), Some("Tyga & Blxst"));
    }

    /// Rows are found the same way regardless of which container holds them, so
    /// continuation pages need no separate path.
    #[test]
    fn reads_rows_from_a_continuation_page_too() {
        let json = continuation_page(vec![row("Later", "v101", "Someone", None)]);
        let tracks = parse_page(&json, &playlist());
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].video_id, "v101");
    }

    #[test]
    fn maps_a_row_onto_a_track() {
        let json = first_page(vec![row("yawa", "v1", "Fireboy DML", Some("UC1"))]);
        let t = &parse_page(&json, &playlist())[0];
        assert_eq!(t.video_id, "v1");
        assert_eq!(t.artist.as_deref(), Some("Fireboy DML"));
        assert_eq!(t.album.as_deref(), Some("Some Album"));
        assert_eq!(t.duration_secs, Some(181.0));
        assert_eq!(t.thumbnail.as_deref(), Some("big.jpg"));
        assert_eq!(t.playlist_id, "VLPL1");
        assert_eq!(t.playlist_title, "archive");
        assert!(t.is_available);
    }

    /// Sets the `musicVideoType` on a row.
    /// Spelled out by hand on purpose — building it from [`VIDEO_TYPE_PATH`]
    /// would make this agree with the parser by construction and assert nothing.
    /// It is an independent restatement of the shape, checked against a captured
    /// response by `real_rows_from_a_captured_response_report_their_video_type`.
    ///
    /// The `watchEndpoint` level below is the one the old parser omitted.
    fn with_type(mut row: Value, video_type: &str) -> Value {
        row["musicResponsiveListItemRenderer"]["overlay"] = json!({
            "musicItemThumbnailOverlayRenderer": { "content": { "musicPlayButtonRenderer": {
                "playNavigationEndpoint": { "watchEndpoint": {
                    "watchEndpointMusicSupportedConfigs": {
                        "watchEndpointMusicConfig": { "musicVideoType": video_type } } } } } } }
        });
        row
    }

    /// Drives whether the player offers a video toggle. A song's "video" is a
    /// square still — the cover art again — so promising footage there would
    /// deliver a photograph.
    #[test]
    fn only_real_videos_report_having_video() {
        let json = first_page(vec![
            with_type(row("A song", "v1", "Artist", None), "MUSIC_VIDEO_TYPE_ATV"),
            with_type(row("A music video", "v2", "Artist", None), "MUSIC_VIDEO_TYPE_OMV"),
            with_type(row("A user video", "v3", "Artist", None), "MUSIC_VIDEO_TYPE_UGC"),
        ]);
        let tracks = parse_page(&json, &playlist());
        assert_eq!(
            tracks
                .iter()
                .map(|t| (t.title.as_str(), t.has_video))
                .collect::<Vec<_>>(),
            [("A song", false), ("A music video", true), ("A user video", true)]
        );
    }

    /// No type at all reports no video — claiming footage we haven't seen would
    /// show a toggle that resolves to a still. Note this row is still *kept*:
    /// `is_music` reads the same `None` and answers the opposite way.
    #[test]
    fn a_row_without_a_video_type_reports_no_video() {
        let tracks = parse_page(&first_page(vec![row("Plain", "v1", "A", None)]), &playlist());
        assert_eq!(tracks.len(), 1, "an unknown type must not drop the row");
        assert!(!tracks[0].has_video);
    }

    /// The test the old suite was missing, and the reason it shipped broken.
    ///
    /// Every hand-built `json!` fixture here is written from the same belief as
    /// the parser, so when that belief was wrong — a `/watchEndpoint` dropped
    /// from the pointer — fixture and parser were wrong *together* and agreed.
    /// The suite was green while `has_video` was false for all 1192 tracks in
    /// the user's library, including 148 titled "(Official Music Video)".
    ///
    /// These four rows are real, captured from a live playlist response, trimmed
    /// to the subtrees the parser reads but otherwise untouched. They cannot
    /// share a mistake with us because YouTube wrote them.
    #[test]
    fn real_rows_from_a_captured_response_report_their_video_type() {
        let json: Value =
            serde_json::from_str(include_str!("../tests/fixtures/playlist_rows.json")).unwrap();
        let tracks = parse_page(&json, &playlist());

        let seen: Vec<(&str, bool)> = tracks
            .iter()
            .map(|t| (t.title.as_str(), t.has_video))
            .collect();
        assert_eq!(
            seen,
            [
                ("Fly N Ghetto", false),                                  // ATV — a still
                ("Kigwa Leero", false),                                   // no type at all
                ("Tyga - Gucci Snake Feat. Desiigner (Audio)", true),     // UGC
                ("Insomnia (Audio)", true),                               // OMV
            ],
            "captured rows must report the video type YouTube actually sent"
        );
        assert!(
            tracks.iter().any(|t| t.has_video),
            "a response containing real videos must yield at least one — \
             an all-false result is the signature of a pointer that never matches"
        );
    }

    #[test]
    fn drops_podcasts_and_uploads_but_keeps_videos() {
        let episode = with_type(
            row("An episode", "e1", "A show", None),
            "MUSIC_VIDEO_TYPE_PODCAST_EPISODE",
        );
        let video = with_type(row("A music video", "m1", "An artist", None), "MUSIC_VIDEO_TYPE_OMV");
        let json = first_page(vec![episode, video]);
        let tracks = parse_page(&json, &playlist());
        assert_eq!(
            tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["A music video"]
        );
    }

    #[test]
    fn skips_deleted_and_id_less_rows_without_failing() {
        let json = first_page(vec![
            row("Song deleted", "v1", "x", None),
            json!({ "musicResponsiveListItemRenderer": { "nonsense": true } }),
            row("Good", "v2", "y", None),
        ]);
        let tracks = parse_page(&json, &playlist());
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "Good");
    }

    #[test]
    fn marks_greyed_out_rows_unavailable_rather_than_dropping_them() {
        let mut greyed = row("Blocked", "v1", "x", None);
        greyed["musicResponsiveListItemRenderer"]["musicItemRendererDisplayPolicy"] =
            json!(GREY_OUT);
        let tracks = parse_page(&first_page(vec![greyed]), &playlist());
        assert_eq!(tracks.len(), 1);
        assert!(!tracks[0].is_available);
    }

    #[test]
    fn a_page_with_no_rows_is_empty_not_an_error() {
        assert!(parse_page(&json!({ "contents": {} }), &playlist()).is_empty());
        assert!(parse_page(&Value::Null, &playlist()).is_empty());
    }
}
