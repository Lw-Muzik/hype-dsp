//! Movies — public-domain / Creative-Commons films from the **Internet
//! Archive** (archive.org). Its `advancedsearch.php` and `metadata` endpoints
//! are public and keyless; this is the legal, licensed alternative to a pirate
//! source.
//!
//! Three commands, mirroring the reqwest::blocking + `#[tauri::command(async)]`
//! + shared-client pattern used across `commands/` (see `autoeq.rs`,
//! `account.rs`): [`movies_browse`] (one collection's popular films),
//! [`movies_search`] (across the whitelisted collections below), and
//! [`movies_resolve`] (an identifier's metadata → playable SD/HD streams).
//!
//! Playback needs no proxy: `GET /download/<id>/<file>` 302s to a CDN node that
//! serves Range requests, so a plain `<video src>` in the webview plays it
//! directly — the same reasoning `tv_stream_url` documents for why *its*
//! proxy exists (headers/CORS/CSP) doesn't apply here.

use std::sync::OnceLock;
use std::time::Duration;

use hm_core::IpcError;
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "HypeMuzik/1.0 (+https://github.com/Lw-Muzik)";
const SEARCH_URL: &str = "https://archive.org/advancedsearch.php";
const METADATA_URL: &str = "https://archive.org/metadata";
const DOWNLOAD_URL: &str = "https://archive.org/download";
const POSTER_URL: &str = "https://archive.org/services/img";

/// Default page size for a browse/search request.
const DEFAULT_ROWS: u32 = 30;

/// One confirmed, non-empty collection (curl-verified `numFound > 0`).
/// `film_noir` / `comedy_films` / `documentaryfilms` were tried and return
/// zero — deliberately left out. Add another only after verifying it live.
///
/// `label` isn't read by any command (the frontend owns its own shelf labels,
/// listed in the same order) — it's kept here so this table stays the single
/// place the confirmed collections are documented, label included.
#[allow(dead_code)]
pub struct CollectionInfo {
    pub id: &'static str,
    pub label: &'static str,
}

pub const COLLECTIONS: &[CollectionInfo] = &[
    CollectionInfo { id: "feature_films", label: "Feature Films" },
    CollectionInfo { id: "SciFi_Horror", label: "Sci-Fi & Horror" },
    CollectionInfo { id: "silent_films", label: "Silent Films" },
    CollectionInfo { id: "classic_cartoons", label: "Classic Cartoons" },
    CollectionInfo { id: "classic_tv", label: "Classic TV" },
    CollectionInfo { id: "prelinger", label: "Prelinger Archives" },
    CollectionInfo { id: "short_films", label: "Short Films" },
];

/// `collection:(a OR b OR ...)` clause restricting a cross-catalog search to
/// collections known to actually contain films.
fn whitelisted_collections_clause() -> String {
    let ids: Vec<&str> = COLLECTIONS.iter().map(|c| c.id).collect();
    format!("collection:({})", ids.join(" OR "))
}

/// The shared blocking client: one connection pool, a sane timeout, and a
/// descriptive User-Agent (archive.org has no key requirement, but a real UA
/// is good API citizenship). Built once and reused, like `tv::` and
/// `radio::`'s clients.
fn client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    })
}

/// Join path segments onto a base URL, percent-encoding each one (spaces and
/// punctuation in an Archive filename become `%20` etc.) rather than
/// string-concatenating a possibly-invalid URL.
fn build_url(base: &str, segments: &[&str]) -> String {
    match reqwest::Url::parse(base) {
        Ok(mut url) => {
            if let Ok(mut segs) = url.path_segments_mut() {
                segs.extend(segments);
            }
            url.to_string()
        }
        // Base is a hardcoded constant, so this never actually happens — but a
        // graceful fallback beats a panic if it ever did.
        Err(_) => {
            let mut s = base.to_string();
            for seg in segments {
                s.push('/');
                s.push_str(seg);
            }
            s
        }
    }
}

fn poster_url(identifier: &str) -> String {
    build_url(POSTER_URL, &[identifier])
}

fn file_url(identifier: &str, name: &str) -> String {
    build_url(DOWNLOAD_URL, &[identifier, name])
}

fn metadata_url(identifier: &str) -> String {
    build_url(METADATA_URL, &[identifier])
}

/* ------------------------------------------------------------- IPC shapes */

/// One film in a browse/search result — enough to render a poster card.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MovieCard {
    pub identifier: String,
    pub title: String,
    pub year: Option<u32>,
    pub poster_url: String,
    pub downloads: Option<u64>,
}

/// One playable rendition of a film.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MovieStream {
    /// `"SD"` or `"HD"`.
    pub label: String,
    pub url: String,
    pub mime: String,
}

/// Full detail for the player screen.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MovieDetail {
    pub identifier: String,
    pub title: String,
    pub year: Option<u32>,
    /// HTML-stripped, or `None` if archive.org had nothing to say.
    pub description: Option<String>,
    pub poster_url: String,
    /// SD first (the spec's default), then HD if the item has one.
    pub streams: Vec<MovieStream>,
}

/* --------------------------------------------------- advancedsearch.php DTOs */

/// `year`/`downloads` are documented as numbers, but archive.org's Solr layer
/// has been known to hand back numeric-looking fields as strings on some
/// legacy items — tolerate either rather than dropping the whole doc.
fn flexible_opt_u32<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    Ok(v.and_then(|v| match v {
        serde_json::Value::Number(n) => n.as_u64().map(|n| n as u32),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }))
}

fn flexible_opt_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    Ok(v.and_then(|v| match v {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }))
}

#[derive(Deserialize)]
struct SearchResponse {
    response: SearchInner,
}

#[derive(Deserialize)]
struct SearchInner {
    #[serde(default)]
    docs: Vec<SearchDoc>,
}

#[derive(Deserialize)]
struct SearchDoc {
    identifier: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default, deserialize_with = "flexible_opt_u32")]
    year: Option<u32>,
    #[serde(default, deserialize_with = "flexible_opt_u64")]
    downloads: Option<u64>,
}

fn doc_to_card(d: SearchDoc) -> MovieCard {
    let poster_url = poster_url(&d.identifier);
    let title = d.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| d.identifier.clone());
    MovieCard {
        identifier: d.identifier,
        title,
        year: d.year,
        poster_url,
        downloads: d.downloads,
    }
}

/// `GET advancedsearch.php` for a Solr query, mapped straight to cards.
fn search_movies(q: &str, page: u32, rows: u32) -> Result<Vec<MovieCard>, IpcError> {
    let rows_str = rows.to_string();
    let page_str = page.max(1).to_string();
    let params: Vec<(&str, &str)> = vec![
        ("q", q),
        ("fl[]", "identifier"),
        ("fl[]", "title"),
        ("fl[]", "year"),
        ("fl[]", "downloads"),
        ("sort[]", "downloads desc"),
        ("rows", &rows_str),
        ("page", &page_str),
        ("output", "json"),
    ];
    let resp = client()
        .get(SEARCH_URL)
        .query(&params)
        .send()
        .map_err(|e| IpcError::new("network", format!("Couldn't reach the Internet Archive: {e}")))?;
    if !resp.status().is_success() {
        return Err(IpcError::new(
            "network",
            format!("Internet Archive search returned HTTP {}", resp.status().as_u16()),
        ));
    }
    let body: SearchResponse = resp
        .json()
        .map_err(|e| IpcError::new("parse", format!("Unexpected search response: {e}")))?;
    Ok(body.response.docs.into_iter().map(doc_to_card).collect())
}

/* ---------------------------------------------------------- metadata DTOs */

/// archive.org's `description` field is usually a single string but is
/// occasionally an array of paragraph strings — accept either.
#[derive(Deserialize)]
#[serde(untagged)]
enum DescriptionField {
    One(String),
    Many(Vec<String>),
}

impl DescriptionField {
    fn into_string(self) -> String {
        match self {
            DescriptionField::One(s) => s,
            DescriptionField::Many(v) => v.join("\n"),
        }
    }
}

#[derive(Deserialize, Default)]
struct ArchiveMetadata {
    #[serde(default)]
    title: Option<String>,
    #[serde(default, deserialize_with = "flexible_opt_u32")]
    year: Option<u32>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    description: Option<DescriptionField>,
}

#[derive(Deserialize)]
struct ArchiveFile {
    name: String,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Deserialize)]
struct MetadataResponse {
    #[serde(default)]
    metadata: Option<ArchiveMetadata>,
    #[serde(default)]
    files: Vec<ArchiveFile>,
}

/// The two renditions the spec calls for. `512Kb MPEG4` is the fast-start SD
/// file (the default); `h.264` is the HD file. When there's no dedicated SD
/// file, SD falls back to the HD one rather than coming back empty — and in
/// that case HD isn't listed again as a second, identical entry.
const SD_FORMAT: &str = "512Kb MPEG4";
const HD_FORMAT: &str = "h.264";

fn pick_streams(identifier: &str, files: &[ArchiveFile]) -> Vec<MovieStream> {
    let find = |fmt: &str| files.iter().find(|f| f.format.as_deref() == Some(fmt));
    let sd_file = find(SD_FORMAT).or_else(|| find(HD_FORMAT));
    let hd_file = find(HD_FORMAT);

    let mut streams = Vec::with_capacity(2);
    if let Some(f) = sd_file {
        streams.push(MovieStream {
            label: "SD".to_string(),
            url: file_url(identifier, &f.name),
            mime: "video/mp4".to_string(),
        });
    }
    if let Some(f) = hd_file {
        if sd_file.map(|s| s.name.as_str()) != Some(f.name.as_str()) {
            streams.push(MovieStream {
                label: "HD".to_string(),
                url: file_url(identifier, &f.name),
                mime: "video/mp4".to_string(),
            });
        }
    }
    streams
}

/// The year out of a `YYYY-MM-DD`-shaped `date` field, when `metadata.year`
/// itself was absent.
fn year_from_date(date: &str) -> Option<u32> {
    date.get(0..4).and_then(|s| s.parse().ok())
}

/// Strip HTML tags from an Archive description and decode the handful of
/// entities that show up in practice. Not a full HTML parser — Archive
/// descriptions are simple marked-up prose (`<br>`, `<p>`, the odd `<a>`),
/// never full documents, so a linear tag-stripper is the right amount of
/// machinery for it.
fn strip_html_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for c in input.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(out.trim())
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

/* ----------------------------------------------------------------- commands */

/// One collection's popular films, most-downloaded first.
#[tauri::command(async)]
pub fn movies_browse(collection: String, page: u32) -> Result<Vec<MovieCard>, IpcError> {
    if collection.trim().is_empty() {
        return Err(IpcError::new("invalid", "collection is required"));
    }
    let q = format!("collection:({collection}) AND mediatype:(movies)");
    search_movies(&q, page, DEFAULT_ROWS)
}

/// Search across the whitelisted collections (the ones known to hold films).
#[tauri::command(async)]
pub fn movies_search(query: String, page: u32) -> Result<Vec<MovieCard>, IpcError> {
    let terms = query.trim();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let q = format!(
        "({terms}) AND mediatype:(movies) AND {}",
        whitelisted_collections_clause()
    );
    search_movies(&q, page, DEFAULT_ROWS)
}

/// An identifier's full metadata, mapped to playable SD/HD streams (SD first).
/// `streams` may come back empty for an item that has neither format — the
/// front end shows that as "no playable version" rather than treating it as
/// a network failure.
#[tauri::command(async)]
pub fn movies_resolve(identifier: String) -> Result<MovieDetail, IpcError> {
    let resp = client()
        .get(metadata_url(&identifier))
        .send()
        .map_err(|e| IpcError::new("network", format!("Couldn't reach the Internet Archive: {e}")))?;
    if !resp.status().is_success() {
        return Err(IpcError::new(
            "network",
            format!("Internet Archive returned HTTP {}", resp.status().as_u16()),
        ));
    }
    let body: MetadataResponse = resp
        .json()
        .map_err(|e| IpcError::new("parse", format!("Unexpected metadata response: {e}")))?;
    let meta = body.metadata.unwrap_or_default();
    let title = meta
        .title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| identifier.clone());
    let year = meta.year.or_else(|| meta.date.as_deref().and_then(year_from_date));
    let description = meta
        .description
        .map(|d| strip_html_tags(&d.into_string()))
        .filter(|s| !s.is_empty());
    let streams = pick_streams(&identifier, &body.files);

    Ok(MovieDetail {
        poster_url: poster_url(&identifier),
        identifier,
        title,
        year,
        description,
        streams,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, format: &str) -> ArchiveFile {
        ArchiveFile { name: name.to_string(), format: Some(format.to_string()) }
    }

    #[test]
    fn parses_search_docs_into_movie_cards() {
        let json = r#"{
            "response": {
                "docs": [
                    { "identifier": "night_of_the_living_dead", "title": "Night of the Living Dead", "year": 1968, "downloads": 500000 },
                    { "identifier": "no_year_or_downloads" }
                ]
            }
        }"#;
        let parsed: SearchResponse = serde_json::from_str(json).unwrap();
        let cards: Vec<MovieCard> = parsed.response.docs.into_iter().map(doc_to_card).collect();
        assert_eq!(
            cards[0],
            MovieCard {
                identifier: "night_of_the_living_dead".to_string(),
                title: "Night of the Living Dead".to_string(),
                year: Some(1968),
                poster_url: "https://archive.org/services/img/night_of_the_living_dead".to_string(),
                downloads: Some(500_000),
            }
        );
        // Missing title/year/downloads must never crash the doc — it degrades
        // to the identifier as a title and nulls for the rest.
        assert_eq!(
            cards[1],
            MovieCard {
                identifier: "no_year_or_downloads".to_string(),
                title: "no_year_or_downloads".to_string(),
                year: None,
                poster_url: "https://archive.org/services/img/no_year_or_downloads".to_string(),
                downloads: None,
            }
        );
    }

    #[test]
    fn tolerates_year_and_downloads_sent_as_strings() {
        let json = r#"{
            "response": {
                "docs": [
                    { "identifier": "quirky_doc", "title": "Quirky", "year": "1955", "downloads": "42" }
                ]
            }
        }"#;
        let parsed: SearchResponse = serde_json::from_str(json).unwrap();
        let card = doc_to_card(parsed.response.docs.into_iter().next().unwrap());
        assert_eq!(card.year, Some(1955));
        assert_eq!(card.downloads, Some(42));
    }

    #[test]
    fn picks_sd_then_hd_when_both_present() {
        let files = vec![
            file("movie.mp4", "h.264"),
            file("movie_512kb.mp4", "512Kb MPEG4"),
            file("movie.ogv", "Ogg Video"),
        ];
        let streams = pick_streams("ident", &files);
        assert_eq!(
            streams,
            vec![
                MovieStream {
                    label: "SD".to_string(),
                    url: "https://archive.org/download/ident/movie_512kb.mp4".to_string(),
                    mime: "video/mp4".to_string(),
                },
                MovieStream {
                    label: "HD".to_string(),
                    url: "https://archive.org/download/ident/movie.mp4".to_string(),
                    mime: "video/mp4".to_string(),
                },
            ]
        );
    }

    #[test]
    fn sd_falls_back_to_hd_file_without_duplicating_it() {
        let files = vec![file("movie.mp4", "h.264")];
        let streams = pick_streams("ident", &files);
        // Only one entry: SD borrows the h.264 file, and HD doesn't repeat it.
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].label, "SD");
        assert_eq!(streams[0].url, "https://archive.org/download/ident/movie.mp4");
    }

    #[test]
    fn no_playable_file_yields_no_streams() {
        let files = vec![file("movie.ogv", "Ogg Video"), file("subs.srt", "SubRip")];
        assert!(pick_streams("ident", &files).is_empty());
    }

    #[test]
    fn file_urls_percent_encode_spaces() {
        let files = vec![file("My Movie (1955).mp4", "h.264")];
        let streams = pick_streams("ident", &files);
        assert_eq!(streams[0].url, "https://archive.org/download/ident/My%20Movie%20(1955).mp4");
    }

    #[test]
    fn strips_tags_and_decodes_entities() {
        assert_eq!(
            strip_html_tags("<p>A film about <b>space &amp; time</b>.</p><br/>"),
            "A film about space & time."
        );
        assert_eq!(strip_html_tags("Plain text, no markup"), "Plain text, no markup");
        assert_eq!(strip_html_tags("  <i>trim me</i>  "), "trim me");
    }

    #[test]
    fn year_from_date_reads_the_leading_four_digits() {
        assert_eq!(year_from_date("1955-01-01T00:00:00Z"), Some(1955));
        assert_eq!(year_from_date("19"), None);
        assert_eq!(year_from_date(""), None);
    }

    #[test]
    fn parses_full_metadata_response_fixture() {
        let json = r#"{
            "metadata": {
                "title": "A Sample Film",
                "date": "1949-06-01T00:00:00Z",
                "description": "<p>Some <i>history</i>.</p>"
            },
            "files": [
                { "name": "a_sample_film.mp4", "format": "h.264" },
                { "name": "a_sample_film_512kb.mp4", "format": "512Kb MPEG4" },
                { "name": "a_sample_film.ogv", "format": "Ogg Video" }
            ]
        }"#;
        let body: MetadataResponse = serde_json::from_str(json).unwrap();
        let meta = body.metadata.unwrap();
        assert_eq!(meta.title.as_deref(), Some("A Sample Film"));
        assert_eq!(meta.year, None); // no explicit "year" field in this fixture
        assert_eq!(year_from_date(meta.date.as_deref().unwrap()), Some(1949));
        assert_eq!(strip_html_tags(&meta.description.unwrap().into_string()), "Some history.");
        let streams = pick_streams("a_sample_film", &body.files);
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].label, "SD");
        assert_eq!(streams[1].label, "HD");
    }

    #[test]
    fn description_array_form_is_joined() {
        let json = r#"{ "metadata": {}, "files": [] }"#;
        let body: MetadataResponse = serde_json::from_str(json).unwrap();
        assert!(body.metadata.unwrap().description.is_none());

        let json_many = r#"{
            "metadata": { "description": ["First paragraph.", "Second paragraph."] },
            "files": []
        }"#;
        let body: MetadataResponse = serde_json::from_str(json_many).unwrap();
        let desc = body.metadata.unwrap().description.unwrap().into_string();
        assert_eq!(desc, "First paragraph.\nSecond paragraph.");
    }

    #[test]
    fn whitelisted_clause_names_every_confirmed_collection() {
        let clause = whitelisted_collections_clause();
        for c in COLLECTIONS {
            assert!(clause.contains(c.id), "clause missing {}", c.id);
        }
        assert!(clause.starts_with("collection:("));
    }
}
