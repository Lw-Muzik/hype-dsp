//! Bundled headphone correction profiles.
//!
//! Genuine AutoEq data (oratory1990 measurements, ParametricEQ format) for a
//! curated set of popular models, embedded at compile time. The production app
//! can grow this to the full AutoEq dataset (thousands of models) by replacing
//! `data/headphones.json` — the format is identical.

use crate::HeadphoneProfile;

const BUNDLED: &str = include_str!("../data/headphones.json");

/// All bundled profiles (parsed once per call; the set is small).
pub fn bundled() -> Vec<HeadphoneProfile> {
    serde_json::from_str(BUNDLED).unwrap_or_default()
}

/// Look up a bundled profile by id.
pub fn get(id: &str) -> Option<HeadphoneProfile> {
    bundled().into_iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_profiles_parse_and_have_bands() {
        let profiles = bundled();
        assert!(profiles.len() >= 20, "expected a curated set of profiles");
        assert!(
            profiles.iter().all(|p| !p.bands.is_empty()),
            "every profile should carry correction bands"
        );
        // Bands use AutoEq filter kinds.
        assert!(profiles
            .iter()
            .flat_map(|p| &p.bands)
            .all(|b| matches!(b.kind.as_str(), "peaking" | "lowShelf" | "highShelf")));
    }
}
