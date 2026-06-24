//! Online AutoEQ database commands.
//!
//! `autoeq_search` is a pure, offline scan of the bundled model index
//! (`hm_core::autoeq_db`). `autoeq_fetch_apply` fetches the *selected* model's
//! `GraphicEQ.txt` from the AutoEq GitHub repo and feeds it into the existing
//! GraphicEQ import pipeline. The fetch runs on Tauri's command thread pool
//! (`#[tauri::command(async)]` on a sync fn — like `lyrics_fetch`), so the
//! blocking HTTP call never touches the UI or audio threads.

use std::time::Duration;

use hm_audio::AudioEngine;
use hm_core::{autoeq_db, AutoEqEntry, IpcError};
use tauri::State;

use super::engine::{apply_graphic_curve, EqImportResult};

/// Default maximum number of search results returned to the UI.
const DEFAULT_LIMIT: usize = 50;
/// Only this host + path prefix may be fetched (no SSRF — see `validate_url`).
const ALLOWED_PREFIX: &str = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/";

/// Search the bundled AutoEQ index by model name (instant, offline).
#[tauri::command]
pub fn autoeq_search(query: String, limit: Option<usize>) -> Vec<AutoEqEntry> {
    autoeq_db::search(&query, limit.unwrap_or(DEFAULT_LIMIT))
}

/// Fetch a model's AutoEq `GraphicEQ.txt` and apply it to the 31-band EQ.
///
/// `url` must come from the bundled index (it is validated against the AutoEq
/// raw-content host + path prefix, so the renderer cannot make this fetch an
/// arbitrary host). The request is bounded by a 15 s timeout.
#[tauri::command(async)]
pub fn autoeq_fetch_apply(
    engine: State<'_, AudioEngine>,
    url: String,
) -> Result<EqImportResult, IpcError> {
    validate_url(&url)?;
    let curve = fetch_curve(&url)?;
    apply_graphic_curve(&engine, &curve)
}

/// Reject any URL that is not an AutoEq raw-content curve (SSRF guard).
fn validate_url(url: &str) -> Result<(), IpcError> {
    if url.starts_with(ALLOWED_PREFIX) {
        Ok(())
    } else {
        Err(IpcError::new(
            "invalid",
            "URL is not an AutoEq database curve",
        ))
    }
}

/// Blocking HTTP GET of a GraphicEQ curve (runs on the command thread pool).
fn fetch_curve(url: &str) -> Result<String, IpcError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("HypeMuzik/AutoEq")
        .build()
        .map_err(|e| IpcError::new("network", e.to_string()))?;
    let resp = client
        .get(url)
        .send()
        .map_err(|e| IpcError::new("network", format!("fetch failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(IpcError::new(
            "network",
            format!("AutoEq server returned HTTP {}", resp.status().as_u16()),
        ));
    }
    resp.text()
        .map_err(|e| IpcError::new("network", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_autoeq_urls() {
        assert!(validate_url("https://evil.example.com/x").is_err());
        assert!(validate_url("https://raw.githubusercontent.com/someone/else/x.txt").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("http://raw.githubusercontent.com/jaakkopasanen/AutoEq/x").is_err());
    }

    #[test]
    fn accepts_autoeq_curve_urls() {
        assert!(validate_url(
            "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/oratory1990/harman_over-ear_2018/Sennheiser%20HD%20600/Sennheiser%20HD%20600%20GraphicEQ.txt"
        )
        .is_ok());
    }
}
