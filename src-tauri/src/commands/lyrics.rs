//! Lyrics resolution: a `.lrc` sidecar or embedded tag for local files, then
//! LRCLIB, then the HypeMuzik backend as a last resort. Returns the raw lyrics
//! (timestamped LRC when available, else plain text) for the UI to parse and
//! sync.

use std::path::Path;
use std::time::Duration;

use hm_audio::probe_lyrics;
use hm_core::MediaStore;
use serde::Deserialize;
use tauri::State;

use crate::commands::identify::{identify_local, RecognitionResult};

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
/// first for a sidecar / embedded lyrics; otherwise it falls back to the online
/// sources. A local file whose tags are too weak to match online lyrics (no
/// artist, or a filename-as-title) is identified by audio fingerprint first —
/// mirroring the mobile app, which corrects a track's metadata online before
/// looking up its lyrics.
#[tauri::command(async)]
pub fn lyrics_fetch(
    store: State<'_, MediaStore>,
    title: String,
    artist: Option<String>,
    duration_secs: Option<f64>,
    path: Option<String>,
) -> Option<String> {
    let mut title = title;
    let mut artist = artist.unwrap_or_default();
    let local_path = path.as_deref().filter(|p| !p.starts_with("http"));

    // 1–2. Local file: .lrc sidecar, then embedded lyrics.
    if let Some(p) = local_path {
        if let Some(lrc) = read_lrc_sidecar(p) {
            return Some(lrc);
        }
        if let Some(embedded) = probe_lyrics(Path::new(p)) {
            return Some(embedded);
        }
    }

    // 3. Weak local tags can't match online lyrics — identify the track first
    //    (fingerprint → AcoustID/MusicBrainz) so we search with real metadata.
    let mut identified = false;
    if let Some(p) = local_path {
        if metadata_is_weak(&title, &artist, p) {
            if let Some(r) = identify_local(&store, p) {
                apply_recognition(&mut title, &mut artist, &r);
                identified = true;
            }
        }
    }

    // 4. LRCLIB (best source of synced lyrics), then the HypeMuzik backend.
    if let Some(text) = lrclib_search(&title, &artist, duration_secs) {
        return Some(text);
    }
    if let Some(text) = backend_lyrics(&title, &artist) {
        return Some(text);
    }

    // 5. Last resort: a track with seemingly-fine tags that still missed may be
    //    mistagged — identify (if we haven't yet) and retry once with the result.
    if !identified {
        if let Some(p) = local_path {
            if let Some(r) = identify_local(&store, p) {
                apply_recognition(&mut title, &mut artist, &r);
                if let Some(text) = lrclib_search(&title, &artist, duration_secs) {
                    return Some(text);
                }
                if let Some(text) = backend_lyrics(&title, &artist) {
                    return Some(text);
                }
            }
        }
    }

    eprintln!("lyrics: none found for {title:?} / {artist:?}");
    None
}

/// A local file's tags are too weak to search online lyrics when it has no
/// artist or its title is just the file name (the candidates worth identifying).
fn metadata_is_weak(title: &str, artist: &str, path: &str) -> bool {
    if artist.trim().is_empty() {
        return true;
    }
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    title.trim() == stem.trim()
}

/// Overwrite the search title/artist with identified values when present.
fn apply_recognition(title: &mut String, artist: &mut String, r: &RecognitionResult) {
    if let Some(t) = r.title() {
        *title = t.to_string();
    }
    if let Some(a) = r.artist() {
        *artist = a.to_string();
    }
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
    let res = http_client(15)?
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
    let res = http_client(15)?
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

#[cfg(test)]
mod tests {
    use super::metadata_is_weak;

    #[test]
    fn weak_when_artist_missing() {
        assert!(metadata_is_weak("Blinding Lights", "", "/m/song.mp3"));
        assert!(metadata_is_weak("Blinding Lights", "   ", "/m/song.mp3"));
    }

    #[test]
    fn weak_when_title_is_just_the_filename() {
        // Symphonia couldn't read a title tag, so the scan used the file stem.
        assert!(metadata_is_weak(
            "01 - some track",
            "The Weeknd",
            "/m/01 - some track.mp3"
        ));
    }

    #[test]
    fn strong_when_real_title_and_artist() {
        assert!(!metadata_is_weak(
            "Blinding Lights",
            "The Weeknd",
            "/m/01 - blinding lights.mp3"
        ));
    }
}
