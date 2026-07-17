//! Searching YouTube's catalog.
//!
//! # Why this module doesn't use `search_songs` and friends
//!
//! `ytmapi-rs` has a typed helper per filter, and every one of them is the
//! library-blanking bug again. Reading its song parser:
//!
//! ```text
//! let plays = parse_flex_column_item(&mut mrlir, 2, 0)?;
//! ```
//!
//! — the play count is mandatory, at a fixed column. Measured live, column 2 of
//! a *search* row is `"149M plays"` but column 2 of a row on a genre page is the
//! album, and a row that states neither is simply gone. The byline is demanded
//! whole in the same breath:
//!
//! ```text
//! .try_expect("Song result should contain 2 or 3 string fields delimited by ' • '")?
//! ```
//!
//! and the results are gathered with `.collect::<Result<Vec<_>>>()?`, so any one
//! row failing either demand destroys **every** result. That is precisely the
//! failure that showed 483 of 1253 library tracks: one credit with no channel
//! link took a whole playlist with it.
//!
//! So the raw JSON comes back through `json_query` and [`crate::explore`] reads
//! the shelves, where an odd row costs itself and nothing else.
//!
//! # A search page is shelves
//!
//! Confirmed against live responses: a filtered search is one
//! `musicShelfRenderer` titled after its filter, holding the same
//! `musicResponsiveListItemRenderer` rows a playlist is made of; an unfiltered
//! search leads with a `musicCardShelfRenderer`; an artist page mixes a
//! `musicShelfRenderer` of top songs with carousels of albums, singles and
//! videos. All three are the construction [`crate::explore::parse_page`] already
//! walks, so this module owns the *query* side — which filter, and what a filter
//! is — and hands the response straight to that reader.

use crate::explore::{parse_page, ExploreShelf};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which slice of the catalog to search.
///
/// [`SearchFilter::Top`] sends no filter at all, which is what produces the
/// top-result card and YouTube's own mix of shelves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchFilter {
    Top,
    Songs,
    Videos,
    Albums,
    Artists,
    Playlists,
}

impl SearchFilter {
    /// Parses the front end's name for a filter.
    ///
    /// Unknown names fall back to [`SearchFilter::Top`] rather than failing: a
    /// filter we don't recognise should still search, and an error here would
    /// turn a typo into an empty screen.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "songs" => Self::Songs,
            "videos" => Self::Videos,
            "albums" => Self::Albums,
            "artists" => Self::Artists,
            "playlists" => Self::Playlists,
            _ => Self::Top,
        }
    }
}

/// Every shelf of results on a search response.
///
/// The reading is [`crate::explore`]'s because the shapes are identical — see
/// this module's header. Named here because this is the surface it serves, and
/// because what a search *is* belongs next to the queries that ask for one.
pub fn parse_search_page(json: &Value) -> Vec<ExploreShelf> {
    parse_page(json)
}

/// The completions YouTube offers for a partial query.
///
/// Read here rather than through `get_search_suggestions` for the same reason as
/// everything else in this module: that helper navigates to a fixed path and
/// errors when it isn't there, which for a type-ahead means one reshaped
/// response replaces the suggestions with an error dialog.
pub fn parse_suggestions(json: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_suggestions(json, &mut out);
    out.dedup();
    out
}

fn collect_suggestions(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(o) => {
            if let Some(s) = o.get("searchSuggestionRenderer") {
                // A suggestion's runs are its own text split for highlighting —
                // "burna " + bold "boy" — so joining them is the whole of
                // reading one, and reading a single run would return half a word.
                if let Some(text) = crate::nav::join_runs(s.pointer("/suggestion/runs")) {
                    out.push(text);
                }
                return;
            }
            for val in o.values() {
                collect_suggestions(val, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|i| collect_suggestions(i, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::explore::ExploreKind;
    use serde_json::json;

    #[test]
    fn names_map_onto_filters_and_anything_else_searches_everything() {
        assert_eq!(SearchFilter::parse("songs"), SearchFilter::Songs);
        assert_eq!(SearchFilter::parse("Albums"), SearchFilter::Albums);
        assert_eq!(SearchFilter::parse(" artists "), SearchFilter::Artists);
        assert_eq!(SearchFilter::parse("playlists"), SearchFilter::Playlists);
        assert_eq!(SearchFilter::parse("videos"), SearchFilter::Videos);
        assert_eq!(SearchFilter::parse("all"), SearchFilter::Top);
        // A typo must still search, not blank the screen.
        assert_eq!(SearchFilter::parse("sngs"), SearchFilter::Top);
        assert_eq!(SearchFilter::parse(""), SearchFilter::Top);
    }

    /// The envelope a filtered search arrives in — which the walk ignores, and
    /// that is exactly why it survives YouTube moving it.
    fn search_response(shelf: Value) -> Value {
        json!({ "contents": { "tabbedSearchResultsRenderer": { "tabs": [{ "tabRenderer": {
            "content": { "sectionListRenderer": { "contents": [shelf] } }
        }}]}}})
    }

    #[test]
    fn reads_an_artist_result_out_of_the_search_envelope() {
        let json = search_response(json!({ "musicShelfRenderer": {
            "title": { "runs": [{ "text": "Artists" }] },
            "contents": [{ "musicResponsiveListItemRenderer": {
                "flexColumns": [{ "musicResponsiveListItemFlexColumnRenderer": {
                    "text": { "runs": [{ "text": "Burna Boy" }] } } }],
                "navigationEndpoint": { "browseEndpoint": {
                    "browseId": "UCr61sufuLt7_eB7ak1bXHIg",
                    "browseEndpointContextSupportedConfigs": {
                        "browseEndpointContextMusicConfig": {
                            "pageType": "MUSIC_PAGE_TYPE_ARTIST" } }
                }}
            }}]
        }}));
        let shelves = parse_search_page(&json);
        assert_eq!(shelves[0].title, "Artists");
        assert_eq!(shelves[0].items[0].kind, ExploreKind::Artist);
        assert_eq!(shelves[0].items[0].id, "UCr61sufuLt7_eB7ak1bXHIg");
    }

    /// One bad row costs itself. This is the whole reason the typed helpers are
    /// unused: theirs would return nothing at all here.
    #[test]
    fn one_unreadable_row_does_not_cost_the_others() {
        let json = search_response(json!({ "musicShelfRenderer": {
            "title": { "runs": [{ "text": "Songs" }] },
            "contents": [
                { "musicResponsiveListItemRenderer": {
                    "playlistItemData": { "videoId": "v1" },
                    "flexColumns": [{ "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "Good" }] } } }]
                }},
                { "musicResponsiveListItemRenderer": { "nonsense": true } },
                { "musicResponsiveListItemRenderer": {
                    "playlistItemData": { "videoId": "v2" },
                    "flexColumns": [{ "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "Also good" }] } } }]
                }}
            ]
        }}));
        let items = &parse_search_page(&json)[0].items;
        assert_eq!(
            items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
            ["Good", "Also good"]
        );
    }

    /// A row with no play count and a byline of one field — the shape upstream
    /// calls "should contain 2 or 3 string fields delimited by ' • '" and errors
    /// on, taking every other result with it.
    #[test]
    fn a_row_without_a_play_count_or_a_full_byline_is_still_a_result() {
        let json = search_response(json!({ "musicShelfRenderer": {
            "title": { "runs": [{ "text": "Songs" }] },
            "contents": [{ "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": "v1" },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "An odd one" }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "Someone" }] } } }
                ]
            }}]
        }}));
        let items = &parse_search_page(&json)[0].items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "An odd one");
        assert_eq!(items[0].kind, ExploreKind::Song);
        assert_eq!(items[0].duration_secs, None);
    }

    #[test]
    fn a_response_it_understands_nothing_of_is_empty_not_an_error() {
        assert!(parse_search_page(&json!({ "contents": {} })).is_empty());
        assert!(parse_search_page(&Value::Null).is_empty());
        assert!(parse_search_page(&json!("garbage")).is_empty());
        // No results at all: a titled shelf with nothing in it.
        let json = search_response(json!({ "musicShelfRenderer": {
            "title": { "runs": [{ "text": "Songs" }] }, "contents": []
        }}));
        assert!(parse_search_page(&json).is_empty());
    }

    #[test]
    fn reads_suggestions_joining_the_halves_they_arrive_split_into() {
        let json = json!({ "contents": [{ "searchSuggestionsSectionRenderer": { "contents": [
            { "searchSuggestionRenderer": { "suggestion": { "runs": [
                { "text": "burna " }, { "text": "boy", "bold": true }
            ]}}},
            { "searchSuggestionRenderer": { "suggestion": { "runs": [
                { "text": "burna boy last last" }
            ]}}}
        ]}}]});
        assert_eq!(parse_suggestions(&json), ["burna boy", "burna boy last last"]);
    }

    #[test]
    fn suggestions_of_a_shape_it_cannot_read_are_none_not_an_error() {
        assert!(parse_suggestions(&json!({ "contents": [] })).is_empty());
        assert!(parse_suggestions(&Value::Null).is_empty());
        assert!(parse_suggestions(&json!({ "contents": [
            { "searchSuggestionRenderer": { "suggestion": {} } }
        ]}))
        .is_empty());
    }
}
