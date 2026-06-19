//! Cloud music commands (Google Drive / Dropbox): connect, list, play.

use hm_audio::AudioEngine;
use hm_core::IpcError;
use tauri::State;

use crate::cloud::{CloudEntry, CloudProvider, CloudState, CloudStatus};

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

/// Forget the stored tokens for `provider`.
#[tauri::command]
pub fn cloud_disconnect(cloud: State<'_, CloudState>, provider: CloudProvider) {
    cloud.disconnect(provider);
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
