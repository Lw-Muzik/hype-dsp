# src-tauri/src/cloud_list.rs

- CloudListCache · struct · L25-L28 — pub struct CloudListCache
- new · function · L33-L38 — pub fn new(path: PathBuf) -> Self
- map · function · L42-L51 — fn map(&self) -> &Mutex<HashMap<String, Vec<CloudEntry>>>
- warm · function · L56-L58 — pub fn warm(&self)
- get · function · L61-L63 — pub fn get(&self, account_id: &str) -> Option<Vec<CloudEntry>>
- put · function · L66-L73 — pub fn put(&self, account_id: &str, entries: Vec<CloudEntry>)
- clear · function · L76-L83 — pub fn clear(&self, account_id: &str)
- write · function · L85-L92 — fn write(&self, json: Option<String>)
