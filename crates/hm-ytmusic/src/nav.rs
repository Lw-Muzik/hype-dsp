//! Small readers for YouTube's renderer JSON.
//!
//! Shared by the two hand-written parsers ([`crate::explore`] and
//! [`crate::playlist`]), which exist because upstream's equivalents treat these
//! same fields as mandatory and destroy a whole response when one is absent.
//! Both readers here return `Option` for exactly that reason.

use serde_json::Value;

/// A renderer's `runs` array flattened to its text ("Album • A Pass • 2019").
///
/// Joined rather than indexed: YouTube varies the number of runs (a subtitle may
/// be `["Playlist", " • ", "12 songs"]` or a bare `["YouTube Music"]`), and
/// reading a fixed index is what made upstream report `"Made for "` as a
/// playlist's author.
pub(crate) fn join_runs(runs: Option<&Value>) -> Option<String> {
    let joined: String = runs?
        .as_array()?
        .iter()
        .filter_map(|r| r.pointer("/text").and_then(Value::as_str))
        .collect();
    let trimmed = joined.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The largest thumbnail in a `thumbnails` array.
pub(crate) fn best_thumbnail(thumbs: Option<&Value>) -> Option<String> {
    thumbs?
        .as_array()?
        .iter()
        .max_by_key(|t| {
            let w = t.pointer("/width").and_then(Value::as_u64).unwrap_or(0);
            let h = t.pointer("/height").and_then(Value::as_u64).unwrap_or(0);
            w * h
        })?
        .pointer("/url")
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn joins_every_run_in_order() {
        let runs = json!([{ "text": "Album" }, { "text": " • " }, { "text": "A Pass" }]);
        assert_eq!(join_runs(Some(&runs)).as_deref(), Some("Album • A Pass"));
    }

    #[test]
    fn absent_or_empty_runs_are_none_not_an_empty_string() {
        assert_eq!(join_runs(None), None);
        assert_eq!(join_runs(Some(&json!([]))), None);
        assert_eq!(join_runs(Some(&json!([{ "text": "  " }]))), None);
        assert_eq!(join_runs(Some(&json!("not an array"))), None);
    }

    #[test]
    fn picks_the_largest_thumbnail() {
        let thumbs = json!([
            { "url": "small.jpg", "width": 60, "height": 60 },
            { "url": "big.jpg", "width": 544, "height": 544 },
        ]);
        assert_eq!(best_thumbnail(Some(&thumbs)).as_deref(), Some("big.jpg"));
        assert_eq!(best_thumbnail(None), None);
        assert_eq!(best_thumbnail(Some(&json!([]))), None);
    }

    /// Missing dimensions must not panic — they just rank lowest.
    #[test]
    fn tolerates_thumbnails_without_dimensions() {
        let thumbs = json!([{ "url": "a.jpg" }, { "url": "b.jpg", "width": 10, "height": 10 }]);
        assert_eq!(best_thumbnail(Some(&thumbs)).as_deref(), Some("b.jpg"));
    }
}
