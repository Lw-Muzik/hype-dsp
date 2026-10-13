# crates/hm-core/src/error.rs

- HmError · enum · L12-L28 — pub enum HmError
- from · function · L31-L33 — fn from(e: serde_json::Error) -> Self
- from · function · L37-L39 — fn from(e: rusqlite::Error) -> Self
- IpcError · struct · L48-L53 — pub struct IpcError
- new · function · L56-L61 — pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self
- from · function · L65-L73 — fn from(e: HmError) -> Self
