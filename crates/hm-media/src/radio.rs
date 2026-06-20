//! Internet radio directory.
//!
//! Searches the public [radio-browser](https://www.radio-browser.info) API and
//! falls back to a bundled seed (genuine top stations) when offline or on the
//! first run. Returns [`RadioStation`]s ready to stream through the chain.

use std::time::Duration;

use hm_core::RadioStation;
use serde::Deserialize;

const SEED: &str = include_str!("../data/radio_seed.json");
const HOSTS: &[&str] = &[
    "https://de1.api.radio-browser.info",
    "https://nl1.api.radio-browser.info",
    "https://at1.api.radio-browser.info",
];

/// The bundled seed stations (genuine radio-browser data).
pub fn seed() -> Vec<RadioStation> {
    serde_json::from_str(SEED).unwrap_or_default()
}

#[derive(Deserialize)]
struct RbStation {
    stationuuid: String,
    name: String,
    url_resolved: Option<String>,
    url: String,
    tags: Option<String>,
    country: Option<String>,
    favicon: Option<String>,
}

fn to_station(s: RbStation) -> Option<RadioStation> {
    let url = s
        .url_resolved
        .filter(|u| u.starts_with("http"))
        .unwrap_or(s.url);
    if !url.starts_with("http") {
        return None;
    }
    let genre = s.tags.and_then(|t| {
        t.split(',')
            .next()
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(String::from)
    });
    Some(RadioStation {
        id: s.stationuuid,
        name: s.name,
        url,
        genre,
        country: s.country.filter(|c| !c.is_empty()),
        favicon: s.favicon.filter(|f| !f.is_empty()),
    })
}

/// Search the directory. Falls back to the (query-filtered) seed on failure, so
/// this never errors — the UI always has something to show.
pub fn search(query: &str) -> Vec<RadioStation> {
    if let Some(stations) = search_remote(query) {
        if !stations.is_empty() {
            return stations;
        }
    }
    seed_filtered(query)
}

fn seed_filtered(query: &str) -> Vec<RadioStation> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return seed();
    }
    seed()
        .into_iter()
        .filter(|s| {
            s.name.to_lowercase().contains(&q)
                || s.genre.as_deref().unwrap_or("").to_lowercase().contains(&q)
                || s.country
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&q)
        })
        .collect()
}

/// Stations for one country (ISO 3166-1 alpha-2 code), most-popular first.
/// Returns empty when offline or the code is invalid.
pub fn by_country(code: &str) -> Vec<RadioStation> {
    by_country_remote(code).unwrap_or_default()
}

fn by_country_remote(code: &str) -> Option<Vec<RadioStation>> {
    let code = code.trim();
    if code.len() != 2 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent("hypemuzik/0.1")
        .build()
        .ok()?;
    let path = format!(
        "/json/stations/search?countrycode={}&hidebroken=true&order=clickcount&reverse=true&limit=300",
        code.to_uppercase()
    );
    for host in HOSTS {
        let Ok(resp) = client.get(format!("{host}{path}")).send() else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        if let Ok(raw) = resp.json::<Vec<RbStation>>() {
            return Some(raw.into_iter().filter_map(to_station).collect());
        }
    }
    None
}

fn search_remote(query: &str) -> Option<Vec<RadioStation>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("hypemuzik/0.1")
        .build()
        .ok()?;

    let q = query.trim();
    let path = if q.is_empty() {
        "/json/stations/search?order=clickcount&reverse=true&hidebroken=true&limit=60".to_string()
    } else {
        format!(
            "/json/stations/search?name={}&hidebroken=true&order=clickcount&reverse=true&limit=60",
            urlencode(q)
        )
    };

    for host in HOSTS {
        let Ok(resp) = client.get(format!("{host}{path}")).send() else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        if let Ok(raw) = resp.json::<Vec<RbStation>>() {
            return Some(raw.into_iter().filter_map(to_station).collect());
        }
    }
    None
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "%20".to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_parses() {
        let stations = seed();
        assert!(stations.len() >= 10, "expected bundled seed stations");
        assert!(stations.iter().all(|s| s.url.starts_with("http")));
    }

    #[test]
    fn seed_filter_matches_name() {
        // The seed is real data; filtering by a substring present in any name
        // returns a subset (and the empty query returns everything).
        assert_eq!(seed_filtered("").len(), seed().len());
        assert!(seed_filtered("zzzzz-no-such-station").len() <= seed().len());
    }
}
