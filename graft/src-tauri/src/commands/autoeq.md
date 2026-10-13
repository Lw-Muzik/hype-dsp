# src-tauri/src/commands/autoeq.rs

- DEFAULT_LIMIT · constant · L19-L19 — const DEFAULT_LIMIT: usize = 50;
- ALLOWED_PREFIX · constant · L21-L21 — const ALLOWED_PREFIX: &str = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/";
- autoeq_search · function · L25-L27 — pub fn autoeq_search(query: String, limit: Option<usize>) -> Vec<AutoEqEntry>
- autoeq_fetch_apply · function · L35-L42 — pub fn autoeq_fetch_apply(
- validate_url · function · L45-L54 — fn validate_url(url: &str) -> Result<(), IpcError>
- fetch_curve · function · L57-L79 — fn fetch_curve(url: &str) -> Result<String, IpcError>
- tests · module · L82-L100 — mod tests
- rejects_non_autoeq_urls · function · L86-L91 — fn rejects_non_autoeq_urls()
- accepts_autoeq_curve_urls · function · L94-L99 — fn accepts_autoeq_curve_urls()
