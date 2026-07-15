//! World TV directory (iptv-org).
//!
//! The TV analog of [`radio`](crate::radio): browse publicly-available, free-to-air
//! television channels from around the world. We consume iptv-org's pre-built
//! **M3U playlists** rather than its multi-megabyte JSON API — each `#EXTINF`
//! line is a self-contained channel record (name, logo, category, and any
//! required HTTP headers), which mirrors how radio fetches per-country lists.
//!
//! - Browse by country → `…/iptv/countries/{cc}.m3u`
//! - Browse by category → `…/iptv/categories/{id}.m3u`
//! - Search / "all TVs" → `…/iptv/index.m3u` (one ~3 MB global file)
//!
//! Fetched playlists are disk-cached with a TTL so browsing is instant after the
//! first visit and still works (from stale cache, then a bundled seed) offline.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hm_core::{TvCategory, TvChannel, TvCountry};

const SEED: &str = include_str!("../data/tv_seed.json");
const WORLD_COUNTRIES: &str = include_str!("../data/world_countries.json");
const BASE: &str = "https://iptv-org.github.io/iptv";
/// Playlists refresh daily upstream; a 7-day local TTL keeps browsing snappy
/// without going stale for long.
const TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Cap search / global-browse results so the UI stays responsive even though the
/// global index carries tens of thousands of channels.
const MAX_RESULTS: usize = 600;

/// The bundled seed channels (a handful of well-known global channels), used as
/// the offline / first-run fallback for search.
pub fn seed() -> Vec<TvChannel> {
    serde_json::from_str(SEED).unwrap_or_default()
}

/// The world country list for the browse grid (bundled from iptv-org), sorted by
/// name. The frontend renders each flag from the ISO code.
pub fn world_countries() -> Vec<TvCountry> {
    serde_json::from_str(WORLD_COUNTRIES).unwrap_or_default()
}

/// The browsable TV categories (iptv-org `group-title` buckets). Curated to the
/// useful, non-adult set; the id is the slug used in the playlist URL.
pub fn categories() -> Vec<TvCategory> {
    const CATS: &[(&str, &str)] = &[
        ("news", "News"),
        ("sports", "Sports"),
        ("movies", "Movies"),
        ("music", "Music"),
        ("kids", "Kids"),
        ("entertainment", "Entertainment"),
        ("general", "General"),
        ("documentary", "Documentary"),
        ("comedy", "Comedy"),
        ("culture", "Culture"),
        ("education", "Education"),
        ("family", "Family"),
        ("lifestyle", "Lifestyle"),
        ("science", "Science"),
        ("travel", "Travel"),
        ("weather", "Weather"),
        ("business", "Business"),
        ("cooking", "Cooking"),
        ("religious", "Religious"),
        ("series", "Series"),
        ("animation", "Animation"),
        ("classic", "Classic"),
        ("outdoor", "Outdoor"),
        ("relax", "Relax"),
        ("legislative", "Legislative"),
        ("shop", "Shopping"),
    ];
    CATS.iter()
        .map(|(id, name)| TvCategory { id: (*id).into(), name: (*name).into() })
        .collect()
}

/// Every channel for a country (ISO 3166-1 alpha-2), from `countries/{cc}.m3u`.
/// Returns empty when offline with no cache and the code is invalid.
pub fn by_country(code: &str, cache_dir: Option<&Path>) -> Vec<TvChannel> {
    let code = code.trim().to_lowercase();
    if !is_alpha2(&code) {
        return Vec::new();
    }
    let url = format!("{BASE}/countries/{code}.m3u");
    let cache = cache_path(cache_dir, "countries", &code);
    match fetch_text_cached(&url, cache.as_deref()) {
        Some(text) => parse_m3u(&text, Some(&code.to_uppercase())),
        None => Vec::new(),
    }
}

/// Every channel for a category (iptv-org slug), from `categories/{id}.m3u`.
pub fn by_category(id: &str, cache_dir: Option<&Path>) -> Vec<TvChannel> {
    let id = id.trim().to_lowercase();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Vec::new();
    }
    let url = format!("{BASE}/categories/{id}.m3u");
    let cache = cache_path(cache_dir, "categories", &id);
    let mut channels = match fetch_text_cached(&url, cache.as_deref()) {
        Some(text) => parse_m3u(&text, None),
        None => Vec::new(),
    };
    channels.truncate(MAX_RESULTS);
    channels
}

/// Search the global directory by name / category / country. An empty query
/// returns a bounded slice of the whole catalog (so the browse tab shows content
/// immediately). Falls back to the bundled seed when the index is unavailable —
/// like radio, this never errors.
pub fn search(query: &str, cache_dir: Option<&Path>) -> Vec<TvChannel> {
    let index = global_index(cache_dir);
    let all = if index.is_empty() { seed() } else { index };
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return all.into_iter().take(MAX_RESULTS).collect();
    }
    all.into_iter()
        .filter(|c| {
            c.name.to_lowercase().contains(&q)
                || c.group.as_deref().unwrap_or("").to_lowercase().contains(&q)
                || c.country.as_deref().unwrap_or("").to_lowercase().contains(&q)
        })
        .take(MAX_RESULTS)
        .collect()
}

/// The parsed global catalog (`index.m3u`), disk-cached. Empty when unavailable.
fn global_index(cache_dir: Option<&Path>) -> Vec<TvChannel> {
    let cache = cache_path(cache_dir, ".", "index");
    match fetch_text_cached(&format!("{BASE}/index.m3u"), cache.as_deref()) {
        Some(text) => parse_m3u(&text, None),
        None => Vec::new(),
    }
}

// ---------------------------------------------------------------- M3U parsing

/// Parse an extended-M3U playlist into channels. `default_country` is applied
/// when a channel's own metadata doesn't imply one (e.g. per-country lists).
///
/// Handles, per entry:
/// - `#EXTINF:-1 tvg-id="…" tvg-logo="…" group-title="…" http-user-agent="…",Name`
/// - one or more following `#EXTVLCOPT:http-user-agent=…` / `http-referrer=…` lines
/// - the stream URL on the next non-comment line
fn parse_m3u(text: &str, default_country: Option<&str>) -> Vec<TvChannel> {
    let mut out = Vec::new();
    let mut pending: Option<Pending> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            pending = Some(parse_extinf(rest, default_country));
        } else if let Some(opt) = line.strip_prefix("#EXTVLCOPT:") {
            if let Some(p) = pending.as_mut() {
                apply_vlcopt(p, opt);
            }
        } else if line.starts_with('#') {
            // Other directives (#EXTM3U, #EXTGRP, …) — ignore.
            continue;
        } else if let Some(mut p) = pending.take() {
            // A non-comment line following an #EXTINF is the stream URL.
            if line.starts_with("http") {
                p.url = line.to_string();
                if let Some(ch) = p.into_channel() {
                    out.push(ch);
                }
            }
        }
    }
    out
}

/// A channel being assembled across an #EXTINF and its #EXTVLCOPT lines.
struct Pending {
    id: String,
    name: String,
    logo: Option<String>,
    group: Option<String>,
    country: Option<String>,
    user_agent: Option<String>,
    referrer: Option<String>,
    quality: Option<String>,
    url: String,
}

impl Pending {
    fn into_channel(self) -> Option<TvChannel> {
        if !self.url.starts_with("http") {
            return None;
        }
        // A stable id is required for favorites; fall back to the URL.
        let id = if self.id.is_empty() { self.url.clone() } else { self.id };
        Some(TvChannel {
            id,
            name: self.name,
            url: self.url,
            logo: self.logo,
            group: self.group,
            country: self.country,
            user_agent: self.user_agent,
            referrer: self.referrer,
            quality: self.quality,
        })
    }
}

fn parse_extinf(rest: &str, default_country: Option<&str>) -> Pending {
    // `rest` is `-1 attr="v" attr="v",Display Name`. Split on the first comma
    // that ends the attribute list (the display name may itself contain commas).
    let (attrs, name) = match rest.split_once(',') {
        Some((a, n)) => (a, n.trim().to_string()),
        None => (rest, String::new()),
    };

    let id = attr(attrs, "tvg-id").unwrap_or_default();
    let country = attr(attrs, "tvg-country")
        .or_else(|| country_from_tvg_id(&id))
        .or_else(|| default_country.map(str::to_string));

    Pending {
        name: name.clone(),
        logo: attr(attrs, "tvg-logo"),
        group: attr(attrs, "group-title").and_then(first_group),
        country,
        user_agent: attr(attrs, "http-user-agent"),
        referrer: attr(attrs, "http-referrer"),
        quality: quality_from_name(&name),
        id,
        url: String::new(),
    }
}

fn apply_vlcopt(p: &mut Pending, opt: &str) {
    if let Some((k, v)) = opt.split_once('=') {
        let v = v.trim().trim_matches('"').to_string();
        if v.is_empty() {
            return;
        }
        match k.trim() {
            "http-user-agent" => p.user_agent = Some(v),
            "http-referrer" => p.referrer = Some(v),
            _ => {}
        }
    }
}

/// Read a `key="value"` attribute out of an #EXTINF attribute list.
fn attr(attrs: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = attrs.find(&needle)? + needle.len();
    let end = attrs[start..].find('"')? + start;
    let v = attrs[start..end].trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// `group-title` can be multi-valued (`"General;Music"`); take the first.
fn first_group(g: String) -> Option<String> {
    g.split(';').next().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// Derive an ISO alpha-2 country from a tvg-id like `Name.ug@SD` → `UG`.
fn country_from_tvg_id(id: &str) -> Option<String> {
    let base = id.split('@').next().unwrap_or(id);
    let cc = base.rsplit('.').next()?;
    is_alpha2(&cc.to_lowercase()).then(|| cc.to_uppercase())
}

/// Pull a resolution hint (`720p`, `1080p`, …) out of a channel name.
fn quality_from_name(name: &str) -> Option<String> {
    // Scan for a run of digits immediately followed by 'p'.
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'p' || bytes[i] == b'P') {
                return Some(format!("{}p", &name[start..i]));
            }
        } else {
            i += 1;
        }
    }
    None
}

fn is_alpha2(code: &str) -> bool {
    code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic())
}

// -------------------------------------------------------------- fetch + cache

fn cache_path(cache_dir: Option<&Path>, sub: &str, key: &str) -> Option<PathBuf> {
    cache_dir.map(|d| d.join("tv").join(sub).join(format!("{key}.m3u")))
}

/// Return the playlist text: from a fresh disk cache, else fetched (and cached),
/// else a stale cache as a last resort. `None` only when there is nothing at all.
fn fetch_text_cached(url: &str, cache: Option<&Path>) -> Option<String> {
    if let Some(path) = cache {
        if is_fresh(path) {
            if let Ok(text) = fs::read_to_string(path) {
                return Some(text);
            }
        }
    }
    match http_get(url) {
        Some(body) => {
            if let Some(path) = cache {
                if let Some(dir) = path.parent() {
                    let _ = fs::create_dir_all(dir);
                }
                let _ = fs::write(path, &body);
            }
            Some(body)
        }
        // Offline: fall back to whatever we cached before, however old.
        None => cache.and_then(|p| fs::read_to_string(p).ok()),
    }
}

fn is_fresh(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|age| age < TTL)
        .unwrap_or(false)
}

fn http_get(url: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("hypemuzik/0.1")
        .build()
        .ok()?;
    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_parses_and_is_playable() {
        let s = seed();
        assert!(s.len() >= 5, "expected bundled seed channels");
        assert!(s.iter().all(|c| c.url.starts_with("http")));
        assert!(s.iter().all(|c| !c.id.is_empty() && !c.name.is_empty()));
    }

    #[test]
    fn world_countries_are_bundled_and_alpha2() {
        let c = world_countries();
        assert!(c.len() > 150, "expected the full world country list");
        assert!(c.iter().all(|x| is_alpha2(&x.code.to_lowercase())));
        assert!(c.iter().any(|x| x.code == "UG" && x.name == "Uganda"));
    }

    #[test]
    fn categories_are_non_empty_and_slugged() {
        let c = categories();
        assert!(c.iter().any(|x| x.id == "news"));
        assert!(c.iter().all(|x| x.id.chars().all(|ch| ch.is_ascii_lowercase())));
    }

    #[test]
    fn parses_a_full_entry_with_inline_and_vlcopt_headers() {
        let m3u = "#EXTM3U\n\
            #EXTINF:-1 tvg-id=\"1Plus1.ua@SD\" tvg-logo=\"https://l/logo.png\" \
            http-user-agent=\"Mozilla/5.0\" group-title=\"General;Music\",1+1 International (720p)\n\
            #EXTVLCOPT:http-referrer=https://ref.example/\n\
            http://host.example/stream.m3u8\n";
        let chans = parse_m3u(m3u, None);
        assert_eq!(chans.len(), 1);
        let c = &chans[0];
        assert_eq!(c.id, "1Plus1.ua@SD");
        assert_eq!(c.name, "1+1 International (720p)");
        assert_eq!(c.url, "http://host.example/stream.m3u8");
        assert_eq!(c.logo.as_deref(), Some("https://l/logo.png"));
        assert_eq!(c.group.as_deref(), Some("General")); // first of "General;Music"
        assert_eq!(c.country.as_deref(), Some("UA")); // from tvg-id suffix
        assert_eq!(c.user_agent.as_deref(), Some("Mozilla/5.0"));
        assert_eq!(c.referrer.as_deref(), Some("https://ref.example/"));
        assert_eq!(c.quality.as_deref(), Some("720p"));
    }

    #[test]
    fn default_country_applies_when_id_has_none() {
        let m3u = "#EXTINF:-1 tvg-id=\"SomeChannel\" group-title=\"News\",Some Channel\n\
            https://host/stream.m3u8\n";
        let chans = parse_m3u(m3u, Some("UG"));
        assert_eq!(chans[0].country.as_deref(), Some("UG"));
    }

    #[test]
    fn skips_entries_without_a_valid_url_and_malformed_lines() {
        let m3u = "#EXTM3U\n\
            #EXTINF:-1 tvg-id=\"A.us\",Channel A\n\
            not-a-url\n\
            #EXTINF:-1 tvg-id=\"B.us\",Channel B\n\
            https://ok/stream.m3u8\n\
            garbage line with no extinf\n";
        let chans = parse_m3u(m3u, None);
        // Only Channel B has a valid URL; Channel A's non-URL line is dropped.
        assert_eq!(chans.len(), 1);
        assert_eq!(chans[0].name, "Channel B");
    }

    #[test]
    fn id_falls_back_to_url_when_tvg_id_missing() {
        let m3u = "#EXTINF:-1 group-title=\"News\",No Id Channel\n\
            https://host/noid.m3u8\n";
        let chans = parse_m3u(m3u, None);
        assert_eq!(chans[0].id, "https://host/noid.m3u8");
    }

    #[test]
    fn quality_parser_finds_resolution_or_nothing() {
        assert_eq!(quality_from_name("BBC One HD (1080p)").as_deref(), Some("1080p"));
        assert_eq!(quality_from_name("CNN (480p) [Not 24/7]").as_deref(), Some("480p"));
        assert_eq!(quality_from_name("Plain Channel").as_deref(), None);
    }

    #[test]
    fn invalid_country_and_category_codes_return_empty_without_network() {
        assert!(by_country("zzz", None).is_empty());
        assert!(by_country("1", None).is_empty());
        assert!(by_category("", None).is_empty());
    }
}
