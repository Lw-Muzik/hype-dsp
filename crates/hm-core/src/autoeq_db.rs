//! Online AutoEQ database — a bundled model→curve-URL index.
//!
//! The full [AutoEq](https://github.com/jaakkopasanen/AutoEq) results set is
//! large (thousands of `GraphicEQ.txt` files across many measurement sources),
//! so instead of bundling every curve we bundle a **deduped index** (one entry
//! per model, preferring canonical sources) and fetch only the *selected*
//! model's curve live (in the Tauri layer). That keeps search instant + fully
//! offline while only the chosen curve touches the network. The fetched curve
//! string is fed into the existing GraphicEQ import path.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// One headphone entry in the bundled AutoEQ index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoEqEntry {
    /// Display name, e.g. `"Sennheiser HD 600"`.
    pub name: String,
    /// Measurement source/rig folder, e.g. `"oratory1990"`.
    pub source: String,
    /// Raw GitHub URL of this model's `GraphicEQ.txt`.
    pub url: String,
}

/// The bundled, deduped snapshot of the AutoEq results index.
const INDEX_JSON: &str = include_str!("../data/autoeq_index.json");

/// Lazily-parsed bundled index (parsed once on first use).
fn index() -> &'static [AutoEqEntry] {
    static INDEX: OnceLock<Vec<AutoEqEntry>> = OnceLock::new();
    INDEX
        .get_or_init(|| serde_json::from_str(INDEX_JSON).unwrap_or_default())
        .as_slice()
}

/// Number of headphones in the bundled index.
pub fn len() -> usize {
    index().len()
}

/// Returns `true` if the bundled index is empty (e.g. failed to parse).
pub fn is_empty() -> bool {
    index().is_empty()
}

/// Case-insensitive ranked search over the bundled index.
///
/// Every whitespace-separated term in `query` must appear in the name (AND
/// semantics). Results are ranked best-first: exact name match, name starts
/// with the query, a word in the name starts with the first term, then plain
/// substring; ties break by shorter name then alphabetically. Results are
/// capped at `limit`. An empty/whitespace query or `limit == 0` returns `[]`.
pub fn search(query: &str, limit: usize) -> Vec<AutoEqEntry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() || limit == 0 {
        return Vec::new();
    }
    let terms: Vec<&str> = q.split_whitespace().collect();
    let first = terms[0];

    let mut scored: Vec<(u8, &'static AutoEqEntry)> = Vec::new();
    for entry in index() {
        let name = entry.name.to_lowercase();
        if !terms.iter().all(|t| name.contains(t)) {
            continue;
        }
        let rank = if name == q {
            0
        } else if name.starts_with(&q) {
            1
        } else if name.split_whitespace().any(|w| w.starts_with(first)) {
            2
        } else {
            3
        };
        scored.push((rank, entry));
    }

    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.name.len().cmp(&b.1.name.len()))
            .then_with(|| a.1.name.cmp(&b.1.name))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, e)| e.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_parses_and_is_substantial() {
        assert!(len() > 3000, "bundled index should hold thousands (got {})", len());
        assert!(!is_empty());
    }

    #[test]
    fn finds_known_models() {
        for needle in ["Sennheiser HD 600", "Sony WH-1000XM4", "AirPods Pro", "Moondrop Aria"] {
            let hits = search(needle, 10);
            assert!(
                !hits.is_empty(),
                "expected at least one hit for {needle:?}"
            );
            let name = needle.to_lowercase();
            // The query terms must all appear in the top result.
            let top = hits[0].name.to_lowercase();
            for term in name.split_whitespace() {
                assert!(
                    top.contains(term),
                    "top hit {:?} for {needle:?} is missing term {term:?}",
                    hits[0].name
                );
            }
            assert!(hits[0].url.starts_with("https://"));
        }
    }

    #[test]
    fn empty_query_returns_empty() {
        assert!(search("", 50).is_empty());
        assert!(search("   ", 50).is_empty());
    }

    #[test]
    fn limit_is_respected() {
        // "e" appears in a huge number of names; ensure the cap holds.
        let hits = search("e", 5);
        assert_eq!(hits.len(), 5);
        assert!(search("hd", 0).is_empty());
    }

    #[test]
    fn exact_match_outranks_substring() {
        // An exact name must rank above a name that merely contains it.
        let hits = search("Sennheiser HD 600", 20);
        let exact = hits
            .iter()
            .position(|e| e.name == "Sennheiser HD 600")
            .expect("exact HD 600 present");
        let variant = hits.iter().position(|e| e.name.contains("HD 600 (2020)"));
        if let Some(v) = variant {
            assert!(exact < v, "exact match should rank before the (2020) variant");
        }
    }

    #[test]
    fn all_terms_must_match() {
        // A model containing both terms; a nonsense second term excludes everything.
        assert!(search("sennheiser zzqqxx", 50).is_empty());
        assert!(!search("sennheiser hd", 50).is_empty());
    }
}
