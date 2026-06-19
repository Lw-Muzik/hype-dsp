//! Lyrics resolution, mirroring the mobile app's chain: a `.lrc` sidecar or
//! embedded tag for local files, then the HypeMuzik backend, then LRCLIB.
//! Returns the raw lyrics (timestamped LRC when available, else plain text) for
//! the UI to parse and sync.

use std::path::Path;
use std::time::Duration;

use hm_audio::probe_lyrics;
use serde::Deserialize;

const USER_AGENT: &str = "HypeMuzik/1.0 (https://github.com/Lw-Muzik)";

/// HypeMuzik backend base URL (same default + override as the mobile app's
/// `API_BASE_URL`).
fn api_base() -> String {
    std::env::var("HM_API_BASE_URL")
        .or_else(|_| std::env::var("API_BASE_URL"))
        .unwrap_or_else(|_| "http://37.60.225.220:3035".to_string())
}

fn http_client(secs: u64) -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(secs))
        .build()
        .ok()
}

/// One LRCLIB search hit (https://lrclib.net/docs).
#[derive(Deserialize)]
struct LrclibHit {
    #[serde(rename = "trackName")]
    track_name: Option<String>,
    #[serde(rename = "artistName")]
    artist_name: Option<String>,
    duration: Option<f64>,
    instrumental: Option<bool>,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
}

/// Resolve lyrics for the current track. `path` (when a local file) is checked
/// first for a sidecar / embedded lyrics; otherwise it falls back to LRCLIB.
#[tauri::command]
pub fn lyrics_fetch(
    title: String,
    artist: Option<String>,
    duration_secs: Option<f64>,
    path: Option<String>,
) -> Option<String> {
    let artist = artist.unwrap_or_default();

    // 1–2. Local file: .lrc sidecar, then embedded lyrics.
    if let Some(p) = path.as_deref() {
        if !p.starts_with("http") {
            if let Some(lrc) = read_lrc_sidecar(p) {
                return Some(lrc);
            }
            if let Some(embedded) = probe_lyrics(Path::new(p)) {
                return Some(embedded);
            }
        }
    }

    // 3. HypeMuzik backend (same endpoint as the mobile app).
    if let Some(text) = backend_lyrics(&title, &artist) {
        return Some(text);
    }

    // 4. LRCLIB fallback.
    lrclib_search(&title, &artist, duration_secs)
}

/// Fetch raw lyrics from the HypeMuzik backend:
/// `GET {base}/api/v1/songLyrics/{title}/{artist}` (returns the body verbatim).
fn backend_lyrics(title: &str, artist: &str) -> Option<String> {
    if title.trim().is_empty() {
        return None;
    }
    let mut url = reqwest::Url::parse(&api_base()).ok()?;
    url.path_segments_mut()
        .ok()?
        .extend(["api", "v1", "songLyrics", title, artist]);
    let res = http_client(8)?
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let body = res.text().ok()?;
    (!body.trim().is_empty()).then_some(body)
}

/// Read a `.lrc` file sitting next to the audio file (same name).
fn read_lrc_sidecar(audio: &str) -> Option<String> {
    let lrc = Path::new(audio).with_extension("lrc");
    let content = std::fs::read_to_string(lrc).ok()?;
    (!content.trim().is_empty()).then_some(content)
}

/// Search LRCLIB and pick the best hit by title/artist/duration, preferring
/// synced lyrics. Mirrors the mobile app's scoring.
fn lrclib_search(title: &str, artist: &str, duration_secs: Option<f64>) -> Option<String> {
    if title.trim().is_empty() {
        return None;
    }
    let query = if artist.is_empty() {
        title.to_string()
    } else {
        format!("{title} {artist}")
    };
    let url = reqwest::Url::parse_with_params(
        "https://lrclib.net/api/search",
        &[("q", query.as_str())],
    )
    .ok()?;
    let res = http_client(10)?
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let hits: Vec<LrclibHit> = res.json().ok()?;

    let title_l = title.to_lowercase();
    let title_l = title_l.trim();
    let artist_l = artist.to_lowercase();
    let artist_l = artist_l.trim();

    let mut best: Option<&LrclibHit> = None;
    let mut best_score = -1i32;
    for h in &hits {
        if h.instrumental == Some(true) {
            continue;
        }
        let rt = h.track_name.as_deref().unwrap_or("").to_lowercase();
        let ra = h.artist_name.as_deref().unwrap_or("").to_lowercase();

        let mut score = 0;
        // Title must match.
        if rt == title_l {
            score += 10;
        } else if rt.contains(title_l) || title_l.contains(&rt) {
            score += 5;
        } else {
            continue;
        }
        // Artist must match when we know it.
        if !artist_l.is_empty() {
            if ra == artist_l {
                score += 10;
            } else if ra.contains(artist_l) || artist_l.contains(&ra) {
                score += 5;
            } else {
                continue;
            }
        }
        // Duration proximity.
        if let (Some(d), Some(rd)) = (duration_secs, h.duration) {
            let diff = (rd - d).abs();
            if diff <= 2.0 {
                score += 5;
            } else if diff <= 5.0 {
                score += 3;
            } else if diff <= 10.0 {
                score += 1;
            }
        }
        if h.synced_lyrics.is_some() {
            score += 3;
        }
        if score > best_score {
            best_score = score;
            best = Some(h);
        }
    }

    best.and_then(|h| {
        h.synced_lyrics
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| h.plain_lyrics.clone().filter(|s| !s.trim().is_empty()))
    })
}
