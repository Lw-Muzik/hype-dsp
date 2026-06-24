//! `hm-core` — the shared vocabulary of HypeMuzik.
//!
//! This crate owns the data types that cross every boundary in the app: the
//! DSP/UI parameter model, presets and headphone profiles, the real-time meter
//! and spectrum frames pushed to the UI, and the licensing seam. It has no I/O
//! and no platform dependencies so it can be shared freely by the audio engine,
//! the media subsystems, and the Tauri layer alike.
//!
//! Every public type here is `serde`-serializable and is mirrored exactly by a
//! TypeScript interface in `src/lib/types.ts`. When a type changes on one side,
//! it must change on the other — they are a single contract expressed twice.

pub mod autoeq_db;
pub mod chain_presets;
pub mod error;
pub mod graphic_eq_import;
pub mod headphones;
pub mod license;
pub mod media_store;
pub mod presets;
pub mod store;
pub mod types;

pub use autoeq_db::AutoEqEntry;
pub use chain_presets::{ChainPreset, ChainPresetStore};
pub use error::{HmError, IpcError};
pub use graphic_eq_import::{interpolate_to_iso_bands, parse_graphic_eq, recommended_preamp};
pub use license::{LicenseError, LicenseMock, LicenseService, LicenseStatus};
pub use media_store::MediaStore;
pub use store::PresetStore;
pub use types::*;

/// Human-facing application metadata, surfaced to the UI on startup so the
/// front end never hardcodes the product name or version.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    /// The DSP engine schema revision. Bumped when [`types::EngineState`]
    /// changes shape, so the UI can detect a mismatch against a stale store.
    pub engine_schema: u32,
}

/// The engine parameter schema version. Bump on any breaking change to
/// [`types::EngineState`] or the preset/profile models.
pub const ENGINE_SCHEMA: u32 = 1;

impl AppInfo {
    /// Build [`AppInfo`] from the running crate's name and version.
    pub fn current(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            engine_schema: ENGINE_SCHEMA,
        }
    }
}
