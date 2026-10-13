# src-tauri/src/commands/open_with.rs

- audio_paths · function · L23-L28 — pub fn audio_paths<I: IntoIterator<Item = String>>(paths: I) -> Vec<String>
- PendingOpen · struct · L33-L33 — pub struct PendingOpen(Mutex<Vec<String>>);
- push · function · L38-L45 — pub fn push(&self, paths: impl IntoIterator<Item = String>)
- drain · function · L47-L49 — fn drain(&self) -> Vec<String>
- take_pending_open · function · L56-L58 — pub fn take_pending_open(pending: State<'_, PendingOpen>) -> Vec<String>
- open_files · function · L65-L76 — pub fn open_files(store: State<'_, MediaStore>, paths: Vec<String>) -> Vec<LibraryTrack>
