//! Bundled ViPER4Android DDC preset library — 600+ `.vdc` correction curves
//! shipped with the app so the user can pick one without hunting for files.
//!
//! The presets are embedded at build time as `name → .vdc text`; selecting one
//! resolves its content through [`vdc_to_iso_bands`](crate::vdc_to_iso_bands)
//! exactly like an imported `.vdc`. Pure data — no I/O, no DSP state.

use serde::Deserialize;
use std::sync::OnceLock;

/// One bundled preset: a display name and its raw `.vdc` body.
#[derive(Debug, Clone, Deserialize)]
struct BundledDdc {
    name: String,
    vdc: String,
}

/// The embedded library (a JSON array of `{name, vdc}`), generated from the
/// ViPER4Android DDC preset pack.
const DDC_JSON: &str = include_str!("../data/ddc_presets.json");

fn presets() -> &'static [BundledDdc] {
    static PRESETS: OnceLock<Vec<BundledDdc>> = OnceLock::new();
    PRESETS
        .get_or_init(|| serde_json::from_str(DDC_JSON).unwrap_or_default())
        .as_slice()
}

/// Number of bundled DDC presets.
pub fn len() -> usize {
    presets().len()
}

/// Returns `true` if the bundled library is empty (e.g. failed to parse).
pub fn is_empty() -> bool {
    presets().is_empty()
}

/// All bundled preset names, in their bundled (case-insensitive sorted) order.
pub fn names() -> Vec<String> {
    presets().iter().map(|p| p.name.clone()).collect()
}

/// The raw `.vdc` body of a bundled preset, looked up by exact name.
pub fn get(name: &str) -> Option<&'static str> {
    presets()
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.vdc.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_is_substantial() {
        assert!(len() > 500, "expected the full DDC pack, got {}", len());
        assert!(!is_empty());
    }

    #[test]
    fn every_bundled_preset_resolves_to_finite_bands() {
        // Integrity check on the WHOLE pack: every bundled .vdc must parse and
        // map to 31 finite band gains — no malformed file ships silently.
        for name in names() {
            let body = get(&name).expect("listed name must resolve");
            assert!(body.contains("SR_"), "{name} has no SR_ line");
            let bands = crate::vdc_to_iso_bands(body)
                .unwrap_or_else(|e| panic!("{name} failed to import: {e}"));
            assert!(
                bands.iter().all(|g| g.is_finite()),
                "{name} produced non-finite bands"
            );
        }
    }

    #[test]
    fn unknown_name_is_none() {
        assert!(get("definitely not a real preset name ~~~").is_none());
    }

    #[test]
    fn names_are_unique() {
        let mut all = names();
        let total = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), total, "bundled preset names must be unique");
    }
}
