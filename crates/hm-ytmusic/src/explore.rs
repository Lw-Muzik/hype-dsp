//! Explore — browsing YouTube's own catalog, as opposed to the user's library.
//!
//! # Why this module owns a parser
//!
//! `ytmapi-rs` exposes `get_mood_playlists`, but it is unusable here on two
//! counts, both measured against a real account:
//!
//! * It fails on **25 of 44** categories. Its parser expects a playlist grid or
//!   carousel at `contents/0`; genre pages lead with a *Songs* shelf
//!   (`musicResponsiveListItemRenderer`), so it errors out and returns nothing.
//! * It models a mood page as playlists **only** — it cannot see albums at all.
//!
//! Those two facts coincide exactly: every category carrying albums (~200 each)
//! is a category it cannot parse. So there is no partial path — reading the
//! shelves ourselves is the only way to reach albums, and it simultaneously
//! recovers the 25 broken categories.
//!
//! # One walk, every surface
//!
//! A mood page, an artist page and a search response are the same construction:
//! shelves of items. They differ only in which of the three shelf renderers they
//! reach for and how deeply they bury them, so [`parse_page`] walks for the
//! shelves themselves rather than navigating an envelope. Measured on live
//! responses: a genre page is five `musicCarouselShelfRenderer`s, an artist page
//! mixes one `musicShelfRenderer` with eight carousels, and a filtered search is
//! a single `musicShelfRenderer` — the same reader serves all three, and no
//! surface needs its own path.
//!
//! # Reading by meaning, not by position
//!
//! Where a row states a fact varies by surface: search puts a song's album in
//! flexible column 1, a genre page puts it in column 2, and column 2 of a search
//! row is the play count instead. Every one of those runs, however, carries its
//! own `browseEndpoint` naming what it opens — so a run linking to an album
//! *is* the album, wherever it sits. Reading the link rather than the index is
//! what makes one parser correct on all of them; an index that means "album"
//! here and "plays" there cannot be read safely at all.
//!
//! # The rule that shapes everything here
//!
//! **Nothing in this module may hard-fail.** The bugs that motivated it —
//! upstream's `collect::<Result<_>>()?` blanking a whole library over one odd
//! playlist, its `gridRenderer` demand erroring on an empty album list — are all
//! the same mistake: insisting on a shape and destroying the entire response
//! when reality differs. Here, an unreadable item is skipped, an unknown shelf
//! is skipped, and a page we understand nothing of is *empty*, never an error.
//! YouTube reshaping one genre must cost that shelf, not Explore.

use crate::nav::{best_thumbnail, join_runs};
use crate::parse_duration;
use crate::playlist::{flex_text, has_video, is_music, VIDEO_TYPE_PATH};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// YouTube's own word for what a browse id opens, hung off a `browseEndpoint`.
const PAGE_TYPE: &str =
    "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType";

const ALBUM_PAGE: &str = "MUSIC_PAGE_TYPE_ALBUM";
const PLAYLIST_PAGE: &str = "MUSIC_PAGE_TYPE_PLAYLIST";
const ARTIST_PAGE: &str = "MUSIC_PAGE_TYPE_ARTIST";

/// A card's `musicVideoType`, stated on the watch endpoint it opens. The list-row
/// equivalent is [`VIDEO_TYPE_PATH`], which hangs off the play button instead.
const CARD_VIDEO_TYPE: &str =
    "/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType";

/// The label for the single-item shelf a search leads with. The card states no
/// heading of its own, and titling it after the item it holds would name the
/// result rather than the shelf.
const TOP_RESULT: &str = "Top result";

/// A browsable thing on a catalog page. One shape for every kind: the id plus
/// the kind is what tells us how to open it, and the frontend only renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreItem {
    pub kind: ExploreKind,
    /// What to open, read according to [`ExploreItem::kind`]:
    ///
    /// * [`ExploreKind::Album`] — the `MPREb…` browse id.
    /// * [`ExploreKind::Playlist`] — the **`VL`-prefixed** browse id.
    ///   `GetPlaylistTracksQuery` sends this verbatim as `browseId`, so the bare
    ///   `PL…` form answers HTTP 400. See [`playlist_browse_id`].
    /// * [`ExploreKind::Artist`] — the `UC…` channel id.
    /// * [`ExploreKind::Song`] / [`ExploreKind::Video`] — the video id. These are
    ///   rows rather than cards: they name a thing to play, not a page to open.
    pub id: String,
    pub title: String,
    /// The subtitle runs joined as YouTube wrote them ("Album • A Pass • 2019").
    ///
    /// Deliberately not split into artist/year by run index: fixed-index reads of
    /// this exact field are what broke the library listing (`"Made for "` parsed
    /// as an author). Joining is honest and cannot misattribute.
    pub subtitle: Option<String>,
    pub thumbnail: Option<String>,
    /// The runs that link to an artist page, joined — so a collaboration reads
    /// "Dave, Tems" rather than whichever half sat at the index we picked.
    ///
    /// `None` where the row credits someone without linking them: a co-credit
    /// with no single channel to point at ("Tyga & Blxst") has no artist run at
    /// all, and inventing one from the byline's text is how that becomes wrong
    /// rather than absent.
    #[serde(default)]
    pub artist: Option<String>,
    /// The run that links to an album page, wherever the surface put it.
    #[serde(default)]
    pub album: Option<String>,
    /// Filled for rows that state a running time. Songs on a genre page don't —
    /// they state a view count instead — so this stays `None` rather than
    /// borrowing a number that isn't one.
    #[serde(default)]
    pub duration_secs: Option<f64>,
    /// Whether there's real footage, on the same terms as [`crate::YtTrack`].
    #[serde(default)]
    pub has_video: bool,
}

/// What an [`ExploreItem`] is, which is also how to open it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExploreKind {
    Playlist,
    Album,
    Artist,
    Song,
    Video,
}

/// One shelf on a catalog page ("Featured playlists", "Albums", "Songs", …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreShelf {
    pub title: String,
    pub items: Vec<ExploreItem>,
}

/// Every shelf on a mood, genre, artist or search page.
///
/// Walks for the shelf renderers instead of navigating to them: the first page
/// of a search nests them under `tabbedSearchResultsRenderer`, an artist page
/// under `singleColumnBrowseResultsRenderer`, and a mood page under a plain
/// section list. None of that matters if you look for the shelves themselves.
pub fn parse_page(json: &Value) -> Vec<ExploreShelf> {
    let mut shelves = Vec::new();
    collect_shelves(json, &mut shelves);
    shelves
}

fn collect_shelves(v: &Value, out: &mut Vec<ExploreShelf>) {
    match v {
        Value::Object(o) => {
            // Shelves don't nest, so a hit ends this subtree.
            if let Some(shelf) = o.get("musicCarouselShelfRenderer") {
                push_shelf(carousel_title(shelf), shelf.pointer("/contents"), out);
                return;
            }
            if let Some(shelf) = o.get("musicShelfRenderer") {
                push_shelf(shelf_title(shelf), shelf.pointer("/contents"), out);
                return;
            }
            if let Some(shelf) = o.get("musicCardShelfRenderer") {
                if let Some(parsed) = parse_card_shelf(shelf) {
                    out.push(parsed);
                }
                return;
            }
            for val in o.values() {
                collect_shelves(val, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|i| collect_shelves(i, out)),
        _ => {}
    }
}

/// Files a shelf, unless it has no title or nothing we can open.
///
/// A shelf we can title but not fill is one whose every item we failed to read,
/// which is worth nothing to show and better dropped than rendered empty.
fn push_shelf(title: Option<String>, contents: Option<&Value>, out: &mut Vec<ExploreShelf>) {
    let Some(title) = title else { return };
    let items = parse_items(contents);
    if !items.is_empty() {
        out.push(ExploreShelf { title, items });
    }
}

fn carousel_title(shelf: &Value) -> Option<String> {
    shelf
        .pointer("/header/musicCarouselShelfBasicHeaderRenderer/title/runs/0/text")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn shelf_title(shelf: &Value) -> Option<String> {
    shelf
        .pointer("/title/runs/0/text")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// The single-item shelf a search leads with: the card itself, then the rows it
/// carries underneath — which is what YouTube renders there.
fn parse_card_shelf(shelf: &Value) -> Option<ExploreShelf> {
    let mut items = Vec::new();
    if let Some(card) = parse_card(shelf) {
        items.push(card);
    }
    items.extend(parse_items(shelf.pointer("/contents")));
    (!items.is_empty()).then(|| ExploreShelf {
        title: TOP_RESULT.to_string(),
        items,
    })
}

/// The top-result card. Shaped like neither renderer around it: its title and
/// thumbnail sit at the top level and its destination on `onTap`.
fn parse_card(card: &Value) -> Option<ExploreItem> {
    let endpoint = card
        .pointer("/onTap")
        .or_else(|| card.pointer("/title/runs/0/navigationEndpoint"))?;
    let subtitle_runs = card.pointer("/subtitle/runs");
    open(
        endpoint,
        card.pointer("/title/runs/0/text").and_then(Value::as_str)?,
        join_runs(subtitle_runs),
        best_thumbnail(card.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")),
        std::slice::from_ref(&subtitle_runs).iter().flatten().copied(),
    )
}

fn parse_items(contents: Option<&Value>) -> Vec<ExploreItem> {
    contents
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(parse_content).collect())
        .unwrap_or_default()
}

/// One shelf entry, whichever of the two item renderers holds it. Carousels
/// carry cards, list shelves carry rows, and a genre page's *Songs* carousel
/// carries rows — so which renderer appears is not a property of the shelf.
fn parse_content(v: &Value) -> Option<ExploreItem> {
    if let Some(card) = v.pointer("/musicTwoRowItemRenderer") {
        return parse_item(card);
    }
    parse_row(v.pointer("/musicResponsiveListItemRenderer")?)
}

/// A card: an album, playlist or artist to open, or a music video to play.
fn parse_item(card: &Value) -> Option<ExploreItem> {
    // Cards state their destination twice — on the item and on the title's first
    // run — and the two agree wherever both exist. The item is the reliable one:
    // a music-video card carries a watch endpoint there and nothing at all on
    // the title run.
    let endpoint = card
        .pointer("/navigationEndpoint")
        .or_else(|| card.pointer("/title/runs/0/navigationEndpoint"))?;
    let subtitle_runs = card.pointer("/subtitle/runs");
    open(
        endpoint,
        card.pointer("/title/runs/0/text").and_then(Value::as_str)?,
        join_runs(subtitle_runs),
        best_thumbnail(card.pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")),
        std::slice::from_ref(&subtitle_runs).iter().flatten().copied(),
    )
}

/// Builds an item from whatever `endpoint` opens.
///
/// `byline` is every runs array that may credit an artist or album; each run's
/// own link decides what it is, so the caller hands over all of them without
/// having to know which one carries what.
fn open<'a>(
    endpoint: &Value,
    title: &str,
    subtitle: Option<String>,
    thumbnail: Option<String>,
    byline: impl Iterator<Item = &'a Value> + Clone,
) -> Option<ExploreItem> {
    if let Some(browse) = endpoint.pointer("/browseEndpoint") {
        let id = browse.pointer("/browseId").and_then(Value::as_str)?;
        let kind = classify(id, browse.pointer(PAGE_TYPE).and_then(Value::as_str))?;
        return Some(ExploreItem {
            kind,
            id: match kind {
                ExploreKind::Playlist => playlist_browse_id(id),
                _ => id.to_string(),
            },
            title: title.to_string(),
            subtitle,
            thumbnail,
            artist: linked_runs(byline.clone(), ARTIST_PAGE),
            album: linked_runs(byline, ALBUM_PAGE),
            duration_secs: None,
            has_video: false,
        });
    }

    // A watch endpoint names something to play rather than a page to open.
    let watch = endpoint.pointer("/watchEndpoint")?;
    let video_id = watch.pointer("/videoId").and_then(Value::as_str)?;
    let video_type = watch.pointer(CARD_VIDEO_TYPE).and_then(Value::as_str);
    is_music(video_type).then(|| ExploreItem {
        kind: kind_of(video_type),
        id: video_id.to_string(),
        title: title.to_string(),
        subtitle,
        thumbnail,
        artist: linked_runs(byline.clone(), ARTIST_PAGE),
        album: linked_runs(byline.clone(), ALBUM_PAGE),
        duration_secs: duration_of(byline),
        has_video: has_video(video_type),
    })
}

/// A list row.
///
/// Rows come two ways and which one a shelf holds is not stated anywhere: a
/// *Songs* shelf lists things to play, carrying a video id under
/// `playlistItemData` and no navigation endpoint at all, while an *Artists* or
/// *Albums* shelf lists pages to open, carrying a `browseEndpoint` and no video
/// id. The two are told apart by which of those is present, since neither ever
/// carries the other's.
fn parse_row(row: &Value) -> Option<ExploreItem> {
    let Some(video_id) = row
        .pointer("/playlistItemData/videoId")
        .and_then(Value::as_str)
    else {
        // No video id: a row that names a page rather than a track.
        let subtitle_runs = row.pointer("/subtitle/runs");
        return open(
            row.pointer("/navigationEndpoint")?,
            &flex_text(row, 0)?,
            flex_text(row, 1).or_else(|| join_runs(subtitle_runs)),
            best_thumbnail(row.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")),
            byline(row),
        );
    };

    let title = flex_text(row, 0)?;
    // A removed track keeps its row but not its identity.
    if title == "Song deleted" {
        return None;
    }
    let video_type = row.pointer(VIDEO_TYPE_PATH).and_then(Value::as_str);
    if !is_music(video_type) {
        return None;
    }
    Some(ExploreItem {
        kind: kind_of(video_type),
        id: video_id.to_string(),
        title,
        // Column 1 is what YouTube prints under the title on every surface, and
        // the columns after it vary (an album here, a play count there), so the
        // one column is the honest subtitle and the linked runs below carry the
        // rest.
        subtitle: flex_text(row, 1),
        thumbnail: best_thumbnail(
            row.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"),
        ),
        artist: linked_runs(byline(row), ARTIST_PAGE),
        album: linked_runs(byline(row), ALBUM_PAGE),
        duration_secs: duration_of(byline(row)).or_else(|| fixed_duration(row)),
        has_video: has_video(video_type),
    })
}

/// The runs of every flexible column after the title.
///
/// Column 0 is the title on every surface measured; the rest are the byline, and
/// *which* of them carries what varies (search credits the album in column 1, a
/// genre page in column 2, and a search row's column 2 is the play count). All
/// of them are handed to the link readers, which take only the runs that name
/// themselves — so the variation never has to be modelled.
///
/// The title is excluded rather than searched because it is the one column whose
/// text is arbitrary: a song called "9:41" would otherwise state a duration.
fn byline(row: &Value) -> impl Iterator<Item = &Value> + Clone {
    (1..)
        .map_while(move |col| {
            row.pointer(&format!(
                "/flexColumns/{col}/musicResponsiveListItemFlexColumnRenderer/text/runs"
            ))
        })
}

/// Every run in `byline` whose link opens `page_type`, joined.
///
/// Joined with ", " to match [`crate::join_artists`]: YouTube's own separator
/// runs sit *between* the linked ones and aren't links themselves, so keeping
/// them would mean reading by position again.
fn linked_runs<'a>(byline: impl Iterator<Item = &'a Value>, page_type: &str) -> Option<String> {
    let names: Vec<&str> = byline
        .filter_map(Value::as_array)
        .flatten()
        .filter(|run| {
            run.pointer("/navigationEndpoint/browseEndpoint")
                .and_then(|b| b.pointer(PAGE_TYPE))
                .and_then(Value::as_str)
                == Some(page_type)
        })
        .filter_map(|run| run.pointer("/text").and_then(Value::as_str))
        .filter(|t| !t.trim().is_empty())
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// The byline run that states a running time.
///
/// Requires a colon: a bare number is a year on an album's byline and a view
/// count everywhere else, and [`parse_duration`] would read "2020" as
/// thirty-three minutes. A duration in these bylines is always `M:SS` or
/// `H:MM:SS`, so demanding the separator costs nothing and rules the rest out.
fn duration_of<'a>(byline: impl Iterator<Item = &'a Value>) -> Option<f64> {
    byline
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|run| run.pointer("/text").and_then(Value::as_str))
        .filter(|text| text.contains(':'))
        .find_map(parse_duration)
}

/// Duration as a playlist row states it: in the first fixed column, as runs or a
/// bare string. Search and genre rows have no fixed columns at all.
fn fixed_duration(row: &Value) -> Option<f64> {
    let text = row.pointer("/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text")?;
    join_runs(text.pointer("/runs"))
        .or_else(|| {
            text.pointer("/simpleText")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .as_deref()
        .and_then(parse_duration)
}

/// Song or video, on YouTube's own marker rather than on which shelf or filter
/// produced the row. `ATV` is an audio entity: its "video" is a square still of
/// the cover art, so it is a song. See [`crate::YtTrack::has_video`].
fn kind_of(video_type: Option<&str>) -> ExploreKind {
    if has_video(video_type) {
        ExploreKind::Video
    } else {
        ExploreKind::Song
    }
}

/// What a browse id opens.
///
/// `page_type` is YouTube's own answer and settles it — including when the answer
/// is something this view doesn't offer (a podcast, a user channel), which is a
/// reason to skip the item rather than to go guessing from its prefix. The
/// prefix rule (`MPREb…` album, `VL…` playlist, `UC…` channel) is the fallback
/// for surfaces that state no type.
fn classify(browse_id: &str, page_type: Option<&str>) -> Option<ExploreKind> {
    match page_type {
        Some(ALBUM_PAGE) => Some(ExploreKind::Album),
        Some(PLAYLIST_PAGE) => Some(ExploreKind::Playlist),
        Some(ARTIST_PAGE) => Some(ExploreKind::Artist),
        Some(_) => None,
        None if browse_id.starts_with("MPREb") => Some(ExploreKind::Album),
        None if browse_id.starts_with("VL") => Some(ExploreKind::Playlist),
        None if browse_id.starts_with("UC") => Some(ExploreKind::Artist),
        None => None,
    }
}

/// A playlist browse id, `VL`-prefixed.
///
/// `GetPlaylistTracksQuery` sends `browseId` verbatim; measured live, the bare
/// `PL…` form answers `HTTP 400 Request contains an invalid argument` while the
/// same id prefixed returns its rows. Every catalog surface measured sends the
/// prefix already, so this only ever adds what is missing — and being idempotent
/// is the point: prepending blindly would break the ids that were already right.
fn playlist_browse_id(id: &str) -> String {
    if id.starts_with("VL") {
        id.to_string()
    } else {
        format!("VL{id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn thumb(url: &str, w: u64, h: u64) -> Value {
        json!({ "url": url, "width": w, "height": h })
    }

    fn item(title: &str, browse_id: Option<&str>, subtitle: &[&str]) -> Value {
        let mut nav = json!({});
        if let Some(id) = browse_id {
            nav = json!({ "browseEndpoint": { "browseId": id } });
        }
        json!({
            "musicTwoRowItemRenderer": {
                "title": { "runs": [{ "text": title, "navigationEndpoint": nav }] },
                "subtitle": {
                    "runs": subtitle.iter().map(|t| json!({ "text": t })).collect::<Vec<_>>()
                },
                "thumbnailRenderer": { "musicThumbnailRenderer": { "thumbnail": {
                    "thumbnails": [thumb("small.jpg", 60, 60), thumb("big.jpg", 544, 544)]
                }}}
            }
        })
    }

    fn shelf(header: &str, items: Vec<Value>) -> Value {
        json!({
            "musicCarouselShelfRenderer": {
                "header": { "musicCarouselShelfBasicHeaderRenderer": {
                    "title": { "runs": [{ "text": header }] }
                }},
                "contents": items
            }
        })
    }

    fn page(shelves: Vec<Value>) -> Value {
        json!({ "contents": { "sectionListRenderer": { "contents": shelves } } })
    }

    /// A run that links somewhere, which is how every reader here decides what a
    /// run means.
    fn linked(text: &str, browse_id: &str, page_type: &str) -> Value {
        json!({
            "text": text,
            "navigationEndpoint": { "browseEndpoint": {
                "browseId": browse_id,
                "browseEndpointContextSupportedConfigs": {
                    "browseEndpointContextMusicConfig": { "pageType": page_type }
                }
            }}
        })
    }

    /// A list row, with its byline columns given as raw runs so each test can
    /// place the album wherever the surface it's imitating puts it.
    fn row(title: &str, video_id: &str, cols: Vec<Vec<Value>>, video_type: Option<&str>) -> Value {
        let mut flex = vec![json!({ "musicResponsiveListItemFlexColumnRenderer": {
            "text": { "runs": [{ "text": title }] } } })];
        for runs in cols {
            flex.push(json!({ "musicResponsiveListItemFlexColumnRenderer": {
                "text": { "runs": runs } } }));
        }
        let mut r = json!({
            "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": video_id },
                "flexColumns": flex,
                "thumbnail": { "musicThumbnailRenderer": { "thumbnail": {
                    "thumbnails": [thumb("small.jpg", 60, 60), thumb("big.jpg", 544, 544)]
                }}}
            }
        });
        if let Some(t) = video_type {
            r["musicResponsiveListItemRenderer"]["overlay"] = json!({
                "musicItemThumbnailOverlayRenderer": { "content": { "musicPlayButtonRenderer": {
                    "playNavigationEndpoint": { "watchEndpoint": {
                        "watchEndpointMusicSupportedConfigs": {
                            "watchEndpointMusicConfig": { "musicVideoType": t } } } } } } }
            });
        }
        r
    }

    #[test]
    fn reads_playlists_and_albums_from_their_shelves() {
        let json = page(vec![
            shelf(
                "Featured playlists",
                vec![item("Owambe", Some("VLRDCLAK5uy_abc"), &["YouTube Music"])],
            ),
            shelf(
                "Albums",
                vec![item(
                    "Miracles",
                    Some("MPREb_MKmMXRbMBVr"),
                    &["Album", " • ", "A Pass", " • ", "2019"],
                )],
            ),
        ]);
        let shelves = parse_page(&json);
        assert_eq!(shelves.len(), 2);

        assert_eq!(shelves[0].title, "Featured playlists");
        assert_eq!(shelves[0].items[0].kind, ExploreKind::Playlist);
        // VL kept: browse sends this verbatim.
        assert_eq!(shelves[0].items[0].id, "VLRDCLAK5uy_abc");
        assert_eq!(shelves[0].items[0].thumbnail.as_deref(), Some("big.jpg"));

        assert_eq!(shelves[1].items[0].kind, ExploreKind::Album);
        assert_eq!(shelves[1].items[0].id, "MPREb_MKmMXRbMBVr");
        // Joined, not index-picked.
        assert_eq!(
            shelves[1].items[0].subtitle.as_deref(),
            Some("Album • A Pass • 2019")
        );
    }

    /// The *Songs* shelf a genre page leads with: a carousel of list rows, not
    /// of cards. Which renderer a shelf holds is not a property of the shelf.
    #[test]
    fn reads_a_songs_carousel_of_list_rows() {
        let json = page(vec![shelf(
            "Songs",
            vec![row(
                "Raindance",
                "1qmrq6I_jHI",
                vec![
                    vec![
                        linked("Dave", "UC1", ARTIST_PAGE),
                        json!({ "text": " & " }),
                        linked("Tems", "UC2", ARTIST_PAGE),
                        json!({ "text": " • " }),
                        json!({ "text": "89M views" }),
                    ],
                    vec![linked("The Boy Who Played the Harp", "MPREb_x", ALBUM_PAGE)],
                ],
                Some("MUSIC_VIDEO_TYPE_ATV"),
            )],
        )]);
        let shelves = parse_page(&json);
        assert_eq!(shelves[0].title, "Songs");
        let song = &shelves[0].items[0];
        assert_eq!(song.kind, ExploreKind::Song);
        // A row's id is a video id — there's no browse id to have.
        assert_eq!(song.id, "1qmrq6I_jHI");
        // Both credits, joined — not whichever one an index landed on.
        assert_eq!(song.artist.as_deref(), Some("Dave, Tems"));
        // Column 2 here, column 1 on a search row: found by its link either way.
        assert_eq!(song.album.as_deref(), Some("The Boy Who Played the Harp"));
        // A view count is not a running time.
        assert_eq!(song.duration_secs, None);
        assert!(!song.has_video);
    }

    /// The same reader on a search row, where the album sits one column earlier
    /// and column 2 is a play count instead. Nothing about the row's position
    /// may leak into what it means.
    #[test]
    fn reads_a_search_song_row_whose_album_sits_in_another_column() {
        let json = page(vec![json!({ "musicShelfRenderer": {
            "title": { "runs": [{ "text": "Songs" }] },
            "contents": [row(
                "Common Person",
                "ZmZi1ZuPB9A",
                vec![
                    vec![
                        linked("Burna Boy", "UCr61", ARTIST_PAGE),
                        json!({ "text": " • " }),
                        linked("Love, Damini", "MPREb_If75wr3HkMf", ALBUM_PAGE),
                        json!({ "text": " • " }),
                        json!({ "text": "3:31" }),
                    ],
                    vec![json!({ "text": "149M plays" })],
                ],
                Some("MUSIC_VIDEO_TYPE_ATV"),
            )]
        }})]);
        let song = &parse_page(&json)[0].items[0];
        assert_eq!(song.artist.as_deref(), Some("Burna Boy"));
        assert_eq!(song.album.as_deref(), Some("Love, Damini"));
        assert_eq!(song.duration_secs, Some(211.0));
        // The play count must not be mistaken for anything: it links nowhere.
        assert_eq!(song.subtitle.as_deref(), Some("Burna Boy • Love, Damini • 3:31"));
    }

    /// A music video card: a watch endpoint on the item, nothing on the title
    /// run. The *Music videos* shelf is entirely these.
    #[test]
    fn reads_a_music_video_card_from_its_watch_endpoint() {
        let json = page(vec![shelf(
            "Music videos",
            vec![json!({ "musicTwoRowItemRenderer": {
                "title": { "runs": [{ "text": "Raindance" }] },
                "subtitle": { "runs": [
                    linked("Dave", "UC1", ARTIST_PAGE),
                    { "text": " • " },
                    { "text": "210M views" }
                ]},
                "navigationEndpoint": { "watchEndpoint": {
                    "videoId": "SOJpE1KMUbo",
                    "watchEndpointMusicSupportedConfigs": { "watchEndpointMusicConfig": {
                        "musicVideoType": "MUSIC_VIDEO_TYPE_OMV" } }
                }}
            }})],
        )]);
        let video = &parse_page(&json)[0].items[0];
        assert_eq!(video.kind, ExploreKind::Video);
        assert_eq!(video.id, "SOJpE1KMUbo");
        assert_eq!(video.artist.as_deref(), Some("Dave"));
        assert!(video.has_video);
    }

    /// YouTube's own `pageType` settles what a browse id is, and a type this view
    /// doesn't offer is a reason to skip rather than to guess from the prefix —
    /// a podcast's channel id starts `UC` exactly like an artist's.
    #[test]
    fn a_stated_page_type_beats_the_id_prefix() {
        assert_eq!(classify("UCabc", Some(ARTIST_PAGE)), Some(ExploreKind::Artist));
        assert_eq!(classify("MPREb_x", Some(ALBUM_PAGE)), Some(ExploreKind::Album));
        assert_eq!(
            classify("PL_no_prefix", Some(PLAYLIST_PAGE)),
            Some(ExploreKind::Playlist)
        );
        assert_eq!(classify("UCabc", Some("MUSIC_PAGE_TYPE_PODCAST_SHOW")), None);
        assert_eq!(classify("UCabc", Some("MUSIC_PAGE_TYPE_USER_CHANNEL")), None);
    }

    #[test]
    fn falls_back_to_the_prefix_when_no_type_is_stated() {
        assert_eq!(classify("MPREb_x", None), Some(ExploreKind::Album));
        assert_eq!(classify("VLPL1", None), Some(ExploreKind::Playlist));
        assert_eq!(classify("UCabc", None), Some(ExploreKind::Artist));
        assert_eq!(classify("RDCLAK5uy_x", None), None);
    }

    /// Measured live: the bare `PL…` form answers HTTP 400, the prefixed one
    /// returns rows. Adding it must not double it on the ids that already have it.
    #[test]
    fn playlist_ids_carry_the_vl_prefix_exactly_once() {
        assert_eq!(playlist_browse_id("VLPL1"), "VLPL1");
        assert_eq!(playlist_browse_id("PL1"), "VLPL1");
        assert_eq!(playlist_browse_id("RDCLAK5uy_x"), "VLRDCLAK5uy_x");
    }

    /// A search result that arrives without the prefix must be openable, since
    /// what reaches `GetPlaylistTracksQuery` is this id verbatim.
    #[test]
    fn a_playlist_result_without_the_prefix_gains_one() {
        let json = page(vec![json!({ "musicShelfRenderer": {
            "title": { "runs": [{ "text": "Playlists" }] },
            "contents": [{ "musicResponsiveListItemRenderer": {
                "flexColumns": [{ "musicResponsiveListItemFlexColumnRenderer": {
                    "text": { "runs": [{ "text": "Naija hits" }] } } }],
                "navigationEndpoint": { "browseEndpoint": {
                    "browseId": "PLbare123",
                    "browseEndpointContextSupportedConfigs": {
                        "browseEndpointContextMusicConfig": { "pageType": PLAYLIST_PAGE } }
                }}
            }}]
        }})]);
        let items = &parse_page(&json)[0].items;
        assert_eq!(items[0].kind, ExploreKind::Playlist);
        assert_eq!(items[0].id, "VLPLbare123");
    }

    /// The top-result card: shaped like neither renderer around it, and the only
    /// shelf that states no heading of its own.
    #[test]
    fn reads_the_top_result_card_and_the_rows_beneath_it() {
        let json = page(vec![json!({ "musicCardShelfRenderer": {
            "title": { "runs": [{ "text": "Burna Boy" }] },
            "subtitle": { "runs": [{ "text": "Artist" }, { "text": " • " }, { "text": "712M monthly audience" }] },
            "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                thumb("small.jpg", 60, 60), thumb("big.jpg", 120, 120)
            ]}}},
            "onTap": { "browseEndpoint": {
                "browseId": "UCr61sufuLt7_eB7ak1bXHIg",
                "browseEndpointContextSupportedConfigs": {
                    "browseEndpointContextMusicConfig": { "pageType": ARTIST_PAGE } }
            }},
            "contents": [row("Last Last", "2cNFrajBk0k", vec![vec![linked("Burna Boy", "UCr61", ARTIST_PAGE)]], Some("MUSIC_VIDEO_TYPE_ATV"))]
        }})]);
        let shelves = parse_page(&json);
        assert_eq!(shelves[0].title, "Top result");
        assert_eq!(shelves[0].items[0].kind, ExploreKind::Artist);
        assert_eq!(shelves[0].items[0].id, "UCr61sufuLt7_eB7ak1bXHIg");
        assert_eq!(shelves[0].items[0].thumbnail.as_deref(), Some("big.jpg"));
        // The card's own rows follow it, as YouTube renders them.
        assert_eq!(shelves[0].items[1].kind, ExploreKind::Song);
        assert_eq!(shelves[0].items[1].id, "2cNFrajBk0k");
    }

    /// An item that opens nothing at all — neither endpoint — takes only itself
    /// down, and a shelf left with none vanishes rather than rendering empty.
    #[test]
    fn drops_shelves_whose_items_open_nothing() {
        let json = page(vec![
            shelf("Nothing openable", vec![item("A thing", None, &["Artist"])]),
            shelf(
                "Community playlists",
                vec![item("Naija oldschool", Some("VLPL123"), &["YouTube Music"])],
            ),
        ]);
        assert_eq!(
            parse_page(&json)
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            ["Community playlists"]
        );
    }

    /// One unreadable item must not cost its neighbours — the exact failure that
    /// blanked the library listing.
    #[test]
    fn skips_only_the_items_it_cannot_read() {
        let json = page(vec![shelf(
            "Featured playlists",
            vec![
                item("Good", Some("VLPL1"), &["YouTube Music"]),
                json!({ "musicTwoRowItemRenderer": { "nonsense": true } }),
                json!({ "musicResponsiveListItemRenderer": { "nonsense": true } }),
                item("Also good", Some("VLPL2"), &[]),
            ],
        )]);
        let shelves = parse_page(&json);
        assert_eq!(
            shelves[0]
                .items
                .iter()
                .map(|i| i.title.as_str())
                .collect::<Vec<_>>(),
            ["Good", "Also good"]
        );
        // No runs at all → no subtitle, not an error.
        assert_eq!(shelves[0].items[1].subtitle, None);
    }

    /// Podcast episodes and library uploads aren't what this library is for, and
    /// a row stating so is dropped rather than queued.
    #[test]
    fn drops_podcast_and_upload_rows() {
        let json = page(vec![shelf(
            "Songs",
            vec![
                row("An episode", "e1", vec![vec![]], Some("MUSIC_VIDEO_TYPE_PODCAST_EPISODE")),
                row("An upload", "u1", vec![vec![]], Some("MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK")),
                row("A song", "s1", vec![vec![]], Some("MUSIC_VIDEO_TYPE_ATV")),
            ],
        )]);
        assert_eq!(
            parse_page(&json)[0]
                .items
                .iter()
                .map(|i| i.title.as_str())
                .collect::<Vec<_>>(),
            ["A song"]
        );
    }

    /// A year is a bare number and would read as a duration; a running time in
    /// these bylines always carries a colon.
    #[test]
    fn does_not_mistake_a_year_or_a_view_count_for_a_duration() {
        let runs = json!([{ "text": "Album" }, { "text": " • " }, { "text": "2020" }]);
        assert_eq!(duration_of(std::iter::once(&runs)), None);
        let runs = json!([{ "text": "89M views" }]);
        assert_eq!(duration_of(std::iter::once(&runs)), None);
        let runs = json!([{ "text": "3:31" }]);
        assert_eq!(duration_of(std::iter::once(&runs)), Some(211.0));
    }

    /// A playlist row states its duration in a fixed column instead; search and
    /// genre rows have no fixed columns at all.
    #[test]
    fn reads_a_duration_from_a_fixed_column_when_the_byline_has_none() {
        let mut r = row("Track", "v1", vec![vec![]], Some("MUSIC_VIDEO_TYPE_ATV"));
        r["musicResponsiveListItemRenderer"]["fixedColumns"] = json!([
            { "musicResponsiveListItemFixedColumnRenderer": { "text": { "runs": [{ "text": "3:01" }] } } }
        ]);
        let json = page(vec![shelf("Songs", vec![r])]);
        assert_eq!(parse_page(&json)[0].items[0].duration_secs, Some(181.0));
    }

    /// A credit with no channel to link ("Tyga & Blxst") leaves the artist
    /// absent — which is the truth — rather than fabricating one from the text.
    #[test]
    fn a_credit_without_a_link_leaves_the_artist_absent_not_wrong() {
        let json = page(vec![shelf(
            "Songs",
            vec![row(
                "Chosen",
                "v93",
                vec![vec![json!({ "text": "Tyga & Blxst" })]],
                Some("MUSIC_VIDEO_TYPE_ATV"),
            )],
        )]);
        let song = &parse_page(&json)[0].items[0];
        assert_eq!(song.artist, None);
        // The row itself survives — that's the whole point.
        assert_eq!(song.title, "Chosen");
        assert_eq!(song.subtitle.as_deref(), Some("Tyga & Blxst"));
    }

    #[test]
    fn a_page_it_understands_nothing_of_is_empty_not_an_error() {
        assert!(parse_page(&json!({ "contents": {} })).is_empty());
        assert!(parse_page(&Value::Null).is_empty());
        assert!(parse_page(&page(vec![])).is_empty());
        assert!(parse_page(&json!({ "contents": { "unknownShelfRenderer": { "x": 1 } } })).is_empty());
        assert!(parse_page(&json!([1, "two", null, { "a": [] }])).is_empty());
    }

    #[test]
    fn a_shelf_without_a_header_is_skipped_rather_than_titled_blank() {
        let json = page(vec![json!({
            "musicCarouselShelfRenderer": {
                "contents": [item("Orphan", Some("VLPL1"), &[])]
            }
        })]);
        assert!(parse_page(&json).is_empty());
    }

    #[test]
    fn picks_the_largest_thumbnail() {
        let json = page(vec![shelf(
            "Albums",
            vec![item("A", Some("MPREb_1"), &["Album"])],
        )]);
        assert_eq!(
            parse_page(&json)[0].items[0].thumbnail.as_deref(),
            Some("big.jpg")
        );
    }
}
