# crates/hm-core/src/autoeq_db.rs

- AutoEqEntry · struct · L17-L24 — pub struct AutoEqEntry
- INDEX_JSON · constant · L27-L27 — const INDEX_JSON: &str = include_str!("../data/autoeq_index.json");
- index · function · L30-L35 — fn index() -> &'static [AutoEqEntry]
- INDEX · constant · L31-L31 — static INDEX: OnceLock<Vec<AutoEqEntry>> = OnceLock::new();
- len · function · L38-L40 — pub fn len() -> usize
- is_empty · function · L43-L45 — pub fn is_empty() -> bool
- search · function · L54-L90 — pub fn search(query: &str, limit: usize) -> Vec<AutoEqEntry>
- tests · module · L93-L158 — mod tests
- index_parses_and_is_substantial · function · L97-L100 — fn index_parses_and_is_substantial()
- finds_known_models · function · L103-L122 — fn finds_known_models()
- empty_query_returns_empty · function · L125-L128 — fn empty_query_returns_empty()
- limit_is_respected · function · L131-L136 — fn limit_is_respected()
- exact_match_outranks_substring · function · L139-L150 — fn exact_match_outranks_substring()
- all_terms_must_match · function · L153-L157 — fn all_terms_must_match()
