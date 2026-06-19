//! Track identification: Chromaprint fingerprint → AcoustID lookup → write the
//! recognized title/artist/album back into untagged files. Mirrors the mobile
//! app's fingerprint feature (Chromaprint + AcoustID), with a fill-empty policy
//! so existing tags are never overwritten.

use std::path::Path;
use std::time::Duration;

use hm_audio::{fingerprint_file, probe_track};
use hm_core::{IpcError, LibraryTrack, MediaStore};
use lofty::config::WriteOptions;
use lofty::prelude::{Accessor, TagExt, TaggedFileExt};
use lofty::tag::Tag;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

const USER_AGENT: &str = "HypeMuzik/1.0 (https://github.com/Lw-Muzik)";
/// AcoustID asks for ≤3 requests/sec; pace batch lookups accordingly.
const ACOUSTID_GAP: Duration = Duration::from_millis(340);

/// What identification found (and whether it was written to the file).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionResult {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    score: f64,
    written: bool,
}

/// Progress of a batch identify, on `library:scan_progress` (shared with scan).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    done: usize,
    total: usize,
}

/* ----------------------------------------------------------------- AcoustID */

#[derive(Deserialize)]
struct AcoustidResponse {
    results: Option<Vec<AcoustidResult>>,
}
#[derive(Deserialize)]
struct AcoustidResult {
    #[serde(default)]
    score: f64,
    recordings: Option<Vec<Recording>>,
}
#[derive(Deserialize)]
struct Recording {
    title: Option<String>,
    artists: Option<Vec<NamedEntity>>,
    releasegroups: Option<Vec<ReleaseGroup>>,
}
#[derive(Deserialize)]
struct NamedEntity {
    name: Option<String>,
}
#[derive(Deserialize)]
struct ReleaseGroup {
    title: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

fn acoustid_key() -> String {
    std::env::var("HM_ACOUSTID_KEY")
        .or_else(|_| std::env::var("ACOUSTID_API_KEY"))
        .unwrap_or_else(|_| "r8INVHtWPX".to_string())
}

/// Look up a fingerprint on AcoustID and pick the best-scoring titled recording.
fn acoustid_lookup(fingerprint: &str, duration: u32) -> Option<RecognitionResult> {
    let url = reqwest::Url::parse_with_params(
        "https://api.acoustid.org/v2/lookup",
        &[
            ("client", acoustid_key().as_str()),
            ("duration", &duration.to_string()),
            ("fingerprint", fingerprint),
            // Space-separated → form-encoded to `recordings+releasegroups`.
            ("meta", "recordings releasegroups"),
        ],
    )
    .ok()?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    let res = client.get(url).header("User-Agent", USER_AGENT).send().ok()?;
    if !res.status().is_success() {
        return None;
    }
    let body: AcoustidResponse = res.json().ok()?;

    let mut best: Option<(f64, &Recording)> = None;
    for result in body.results.as_deref().unwrap_or(&[]) {
        let Some(rec) = result
            .recordings
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .find(|r| r.title.as_deref().is_some_and(|t| !t.trim().is_empty()))
        else {
            continue;
        };
        if best.is_none_or(|(s, _)| result.score > s) {
            best = Some((result.score, rec));
        }
    }
    let (score, rec) = best?;

    let artist = rec.artists.as_ref().and_then(|a| {
        let names: Vec<&str> = a.iter().filter_map(|x| x.name.as_deref()).collect();
        (!names.is_empty()).then(|| names.join(", "))
    });
    let album = rec.releasegroups.as_ref().and_then(|rgs| {
        rgs.iter()
            .find(|rg| rg.kind.as_deref() == Some("Album"))
            .or_else(|| rgs.first())
            .and_then(|rg| rg.title.clone())
    });
    Some(RecognitionResult {
        title: rec.title.clone(),
        artist,
        album,
        score,
        written: false,
    })
}

/* ----------------------------------------------------------------- tagging */

fn is_blank(v: Option<std::borrow::Cow<'_, str>>) -> bool {
    v.is_none_or(|s| s.trim().is_empty())
}

/// Write recognized fields into the file, only filling tags that are empty.
/// Returns whether anything was written.
fn write_tags_fill_empty(path: &str, r: &RecognitionResult) -> bool {
    let Ok(tagged) = lofty::read_from_path(path) else {
        return false;
    };
    let tag_type = tagged.primary_tag_type();
    let mut tag = tagged
        .primary_tag()
        .cloned()
        .unwrap_or_else(|| Tag::new(tag_type));

    let mut changed = false;
    if let Some(t) = &r.title {
        if is_blank(tag.title()) {
            tag.set_title(t.clone());
            changed = true;
        }
    }
    if let Some(a) = &r.artist {
        if is_blank(tag.artist()) {
            tag.set_artist(a.clone());
            changed = true;
        }
    }
    if let Some(al) = &r.album {
        if is_blank(tag.album()) {
            tag.set_album(al.clone());
            changed = true;
        }
    }
    if !changed {
        return false;
    }
    tag.save_to_path(path, WriteOptions::default()).is_ok()
}

/// Re-read the file's tags into the library DB so the listing matches the file.
fn refresh_db(store: &MediaStore, path: &str) {
    let p = Path::new(path);
    let (tags, duration) = probe_track(p);
    let filename = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();
    let track = LibraryTrack {
        path: path.to_string(),
        title: tags.title.filter(|t| !t.trim().is_empty()).unwrap_or(filename),
        artist: tags.artist,
        album: tags.album,
        genre: tags.genre,
        duration_secs: duration,
    };
    let _ = store.upsert_tracks(&[track]);
}

/// A track is "missing info" if it has no artist or its title is just the
/// filename — the candidates worth identifying.
fn needs_identify(t: &LibraryTrack) -> bool {
    let no_artist = t.artist.as_deref().is_none_or(|s| s.trim().is_empty());
    let filename = Path::new(&t.path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    no_artist || t.title.trim() == filename.trim()
}

/* ----------------------------------------------------------------- commands */

/// Identify one local track by audio fingerprint and fill in any missing tags.
#[tauri::command]
pub fn identify_track(store: State<'_, MediaStore>, path: String) -> Option<RecognitionResult> {
    let (fingerprint, duration) = fingerprint_file(Path::new(&path))?;
    let mut result = acoustid_lookup(&fingerprint, duration)?;
    result.written = write_tags_fill_empty(&path, &result);
    if result.written {
        refresh_db(&store, &path);
    }
    Some(result)
}

/// Fingerprint + identify every library track that's missing information,
/// filling tags in place. Rate-limited for AcoustID; emits progress; returns
/// the number of tracks successfully tagged.
#[tauri::command]
pub fn library_identify_missing(
    app: tauri::AppHandle,
    store: State<'_, MediaStore>,
) -> Result<usize, IpcError> {
    let missing: Vec<LibraryTrack> = store
        .list_tracks()
        .map_err(IpcError::from)?
        .into_iter()
        .filter(needs_identify)
        .collect();

    let total = missing.len();
    let _ = app.emit("library:scan_progress", Progress { done: 0, total });

    let mut tagged = 0;
    for (i, track) in missing.iter().enumerate() {
        if let Some((fingerprint, duration)) = fingerprint_file(Path::new(&track.path)) {
            if let Some(result) = acoustid_lookup(&fingerprint, duration) {
                if write_tags_fill_empty(&track.path, &result) {
                    refresh_db(&store, &track.path);
                    tagged += 1;
                }
            }
        }
        let _ = app.emit("library:scan_progress", Progress { done: i + 1, total });
        if i + 1 < total {
            std::thread::sleep(ACOUSTID_GAP);
        }
    }
    Ok(tagged)
}
