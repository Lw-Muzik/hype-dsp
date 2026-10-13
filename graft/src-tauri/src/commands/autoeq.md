# src-tauri/src/commands/autoeq.rs

- DEFAULT_LIMIT · constant · L19-L19 — const DEFAULT_LIMIT: usize = 50;
- ALLOWED_PREFIX · constant · L21-L21 — const ALLOWED_PREFIX: &str = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/";
- autoeq_search · function · L25-L27 — pub fn autoeq_search(query: String, limit: Option<usize>) -> Vec<AutoEqEntry>
- autoeq_fetch_apply · function · L35-L44 — pub fn autoeq_fetch_apply(
- validate_url · function · L47-L56 — fn validate_url(url: &str) -> Result<(), IpcError>
- fetch_curve · function · L59-L81 — fn fetch_curve(url: &str) -> Result<String, IpcError>
- tests · module · L84-L102 — mod tests
- rejects_non_autoeq_urls · function · L88-L93 — fn rejects_non_autoeq_urls()
- accepts_autoeq_curve_urls · function · L96-L101 — fn accepts_autoeq_curve_urls()
