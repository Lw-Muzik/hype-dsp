//! Cloud music commands (Google Drive / Dropbox): connect, list, play.

use std::collections::HashMap;

use hm_audio::AudioEngine;
use hm_core::IpcError;
use serde::Serialize;
use tauri::State;

use crate::cloud::{CloudEntry, CloudProvider, CloudState, CloudStatus};
use crate::cloud_list::CloudListCache;
use crate::cloud_meta::{CloudMetaCache, CloudTrackMeta};

/// A flat account-wide audio listing plus whether it was served from the
/// on-disk cache. `fromCache` lets the front-end show it instantly and then
/// quietly re-list in the background, rather than re-fetching a list it just
/// fetched fresh.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAudioPage {
    pub entries: Vec<CloudEntry>,
    pub from_cache: bool,
}

/// Which providers are configured (have credentials) and connected.
#[tauri::command]
pub fn cloud_status(cloud: State<'_, CloudState>) -> CloudStatus {
    cloud.status()
}

/// Run the OAuth flow for `provider` (opens the browser; blocks until the user
/// finishes or it times out).
#[tauri::command(async)]
pub fn cloud_connect(cloud: State<'_, CloudState>, provider: CloudProvider) -> Result<(), IpcError> {
    cloud
        .connect(provider)
        .map_err(|e| IpcError::new("cloud", e))
}

/// Forget the stored tokens for `provider`, and drop its cached listing so a
/// reconnect (possibly a different account) starts clean.
#[tauri::command]
pub fn cloud_disconnect(
    cloud: State<'_, CloudState>,
    list_cache: State<'_, CloudListCache>,
    provider: CloudProvider,
) {
    cloud.disconnect(provider);
    list_cache.clear(provider);
}

/// List the contents of one cloud folder (subfolders + audio files). `folder`
/// is the provider handle, or "" for the account root.
#[tauri::command(async)]
pub fn cloud_list(
    cloud: State<'_, CloudState>,
    provider: CloudProvider,
    folder: String,
) -> Result<Vec<CloudEntry>, IpcError> {
    cloud
        .list(provider, &folder)
        .map_err(|e| IpcError::new("cloud", e))
}

/// Every audio file in the account, flat (all folders) — for the Player's
/// unified library. Mirrors the mobile app's account-wide listing so songs
/// nested in subfolders are included, unlike folder-by-folder `cloud_list`.
///
/// The result is cached on disk per provider: when `refresh` is false a cached
/// listing is returned instantly (no network) so reopening the app is fast;
/// `refresh` (or a cold cache) re-lists from the provider and updates the cache.
#[tauri::command(async)]
pub fn cloud_all_audio(
    cloud: State<'_, CloudState>,
    list_cache: State<'_, CloudListCache>,
    provider: CloudProvider,
    refresh: bool,
) -> Result<CloudAudioPage, IpcError> {
    if !refresh {
        if let Some(entries) = list_cache.get(provider) {
            return Ok(CloudAudioPage {
                entries,
                from_cache: true,
            });
        }
    }
    let entries = cloud
        .all_audio(provider)
        .map_err(|e| IpcError::new("cloud", e))?;
    list_cache.put(provider, entries.clone());
    Ok(CloudAudioPage {
        entries,
        from_cache: false,
    })
}

/// Every cached tag/cover for `provider`, keyed by file id (from the on-disk
/// metadata cache). Lets the library hydrate covers/titles in one call on
/// launch instead of one round-trip per track.
#[tauri::command]
pub fn cloud_cached_metadata(
    cache: State<'_, CloudMetaCache>,
    provider: CloudProvider,
) -> HashMap<String, CloudTrackMeta> {
    cache.snapshot_for(&format!("{provider:?}"))
}

/// Read a cloud track's embedded tags (title/artist/album + cover) by fetching
/// only the file's leading bytes — mirrors the mobile app reading metadata
/// straight off the cloud stream. Cached on disk per file, so it's a one-time
/// download. `name` hints the container format from the file extension.
#[tauri::command(async)]
pub fn cloud_track_metadata(
    cloud: State<'_, CloudState>,
    cache: State<'_, CloudMetaCache>,
    provider: CloudProvider,
    file_id: String,
    name: Option<String>,
) -> Option<CloudTrackMeta> {
    let key = format!("{provider:?}:{file_id}");
    if let Some(hit) = cache.get(&key) {
        return Some(hit);
    }
    let (url, headers) = cloud.stream_target(provider, &file_id).ok()?;
    let ext = name
        .as_deref()
        .and_then(|n| n.rsplit('.').next())
        .filter(|e| !e.is_empty() && e.len() <= 5)
        .map(|e| e.to_ascii_lowercase());

    let meta = hm_audio::fetch_stream_metadata(&url, &headers, ext.as_deref())?;
    let result = CloudTrackMeta {
        title: meta.title,
        artist: meta.artist,
        album: meta.album,
        cover: meta.cover,
    };
    if result.is_useful() {
        cache.put(key, result.clone());
    }
    Some(result)
}

/// Resolve a streamable URL for the file and play it through the chain.
#[tauri::command(async)]
pub fn cloud_play(
    cloud: State<'_, CloudState>,
    engine: State<'_, AudioEngine>,
    provider: CloudProvider,
    file_id: String,
) -> Result<(), IpcError> {
    let (url, headers) = cloud
        .stream_target(provider, &file_id)
        .map_err(|e| IpcError::new("cloud", e))?;
    // Cloud files carry no duration hint; the source learns it from the
    // container (Content-Length + Range) when the server supports it.
    engine.play_stream(url, headers, None).map_err(Into::into)
}
