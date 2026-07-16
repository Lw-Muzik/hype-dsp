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
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A browsable thing on a mood/genre page. One shape for both kinds: the id is
/// what tells us how to open it, and the frontend only renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreItem {
    pub kind: ExploreKind,
    /// Album: the `MPREb…` browse id. Playlist: the **`VL`-prefixed** browse id
    /// — `GetPlaylistTracksQuery` sends this verbatim as `browseId`, so stripping
    /// the prefix earns an HTTP 400.
    pub id: String,
    pub title: String,
    /// The subtitle runs joined as YouTube wrote them ("Album • A Pass • 2019").
    ///
    /// Deliberately not split into artist/year by run index: fixed-index reads of
    /// this exact field are what broke the library listing (`"Made for "` parsed
    /// as an author). Joining is honest and cannot misattribute.
    pub subtitle: Option<String>,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExploreKind {
    Playlist,
    Album,
}

/// One carousel on a mood page ("Featured playlists", "Albums", …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreShelf {
    pub title: String,
    pub items: Vec<ExploreItem>,
}

/// Reads every carousel shelf on a mood/genre page.
///
/// Shelves with no item we can open are dropped rather than shown empty — the
/// *Songs* and *Music videos* shelves land here (list-renderer rows and watch
/// endpoints respectively, neither of which is a browsable id), which is how
/// they're excluded without naming them.
pub fn parse_mood_page(json: &Value) -> Vec<ExploreShelf> {
    let mut shelves = Vec::new();
    collect_shelves(json, &mut shelves);
    shelves
}

fn collect_shelves(v: &Value, out: &mut Vec<ExploreShelf>) {
    match v {
        Value::Object(o) => {
            if let Some(shelf) = o.get("musicCarouselShelfRenderer") {
                if let Some(parsed) = parse_shelf(shelf) {
                    out.push(parsed);
                }
                // Carousels don't nest; don't re-walk this subtree.
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

fn parse_shelf(shelf: &Value) -> Option<ExploreShelf> {
    let title = shelf
        .pointer("/header/musicCarouselShelfBasicHeaderRenderer/title/runs/0/text")
        .and_then(Value::as_str)?
        .to_string();
    let items: Vec<ExploreItem> = shelf
        .pointer("/contents")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|i| parse_item(i.pointer("/musicTwoRowItemRenderer")?))
        .collect();
    (!items.is_empty()).then_some(ExploreShelf { title, items })
}

fn parse_item(mtrir: &Value) -> Option<ExploreItem> {
    let title = mtrir
        .pointer("/title/runs/0/text")
        .and_then(Value::as_str)?
        .to_string();
    // Only a *browse* endpoint is openable. Music-video rows carry a watch
    // endpoint instead and drop out here.
    let id = mtrir
        .pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId")
        .and_then(Value::as_str)?;
    let kind = classify(id)?;
    Some(ExploreItem {
        kind,
        id: id.to_string(),
        title,
        subtitle: join_runs(mtrir.pointer("/subtitle/runs")),
        thumbnail: best_thumbnail(
            mtrir.pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails"),
        ),
    })
}

/// YouTube's browse-id prefixes: `MPREb…` is an album, `VL…` a playlist.
/// Anything else (artists, channels, …) isn't something this view can open.
fn classify(browse_id: &str) -> Option<ExploreKind> {
    if browse_id.starts_with("MPREb") {
        Some(ExploreKind::Album)
    } else if browse_id.starts_with("VL") {
        Some(ExploreKind::Playlist)
    } else {
        None
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
        let shelves = parse_mood_page(&json);
        assert_eq!(shelves.len(), 2);

        assert_eq!(shelves[0].title, "Featured playlists");
        assert_eq!(
            shelves[0].items[0],
            ExploreItem {
                kind: ExploreKind::Playlist,
                // VL kept: browse sends this verbatim.
                id: "VLRDCLAK5uy_abc".into(),
                title: "Owambe".into(),
                subtitle: Some("YouTube Music".into()),
                thumbnail: Some("big.jpg".into()),
            }
        );

        assert_eq!(shelves[1].items[0].kind, ExploreKind::Album);
        assert_eq!(shelves[1].items[0].id, "MPREb_MKmMXRbMBVr");
        // Joined, not index-picked.
        assert_eq!(
            shelves[1].items[0].subtitle.as_deref(),
            Some("Album • A Pass • 2019")
        );
    }

    /// The Songs / Music videos shelves: rows we can't open. They must vanish
    /// quietly, taking their shelf with them, not blank the page.
    #[test]
    fn drops_shelves_whose_items_are_not_browsable() {
        let json = page(vec![
            json!({
                "musicCarouselShelfRenderer": {
                    "header": { "musicCarouselShelfBasicHeaderRenderer": {
                        "title": { "runs": [{ "text": "Songs" }] }
                    }},
                    // list rows, not two-row items
                    "contents": [{ "musicResponsiveListItemRenderer": { "x": 1 } }]
                }
            }),
            shelf(
                "Music videos",
                vec![item("A video", None, &["Artist"])], // watch endpoint → no browseId
            ),
            shelf(
                "Community playlists",
                vec![item("Naija oldschool", Some("VLPL123"), &["YouTube Music"])],
            ),
        ]);
        let shelves = parse_mood_page(&json);
        assert_eq!(
            shelves.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
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
                item("Artist page", Some("UCabc"), &["Artist"]), // not openable
                item("Also good", Some("VLPL2"), &[]),
            ],
        )]);
        let shelves = parse_mood_page(&json);
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

    #[test]
    fn a_page_it_understands_nothing_of_is_empty_not_an_error() {
        assert!(parse_mood_page(&json!({ "contents": {} })).is_empty());
        assert!(parse_mood_page(&Value::Null).is_empty());
        assert!(parse_mood_page(&page(vec![])).is_empty());
    }

    #[test]
    fn a_shelf_without_a_header_is_skipped_rather_than_titled_blank() {
        let json = page(vec![json!({
            "musicCarouselShelfRenderer": {
                "contents": [item("Orphan", Some("VLPL1"), &[])]
            }
        })]);
        assert!(parse_mood_page(&json).is_empty());
    }

    #[test]
    fn picks_the_largest_thumbnail() {
        let json = page(vec![shelf(
            "Albums",
            vec![item("A", Some("MPREb_1"), &["Album"])],
        )]);
        assert_eq!(
            parse_mood_page(&json)[0].items[0].thumbnail.as_deref(),
            Some("big.jpg")
        );
    }
}
