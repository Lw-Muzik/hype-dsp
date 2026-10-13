# src-tauri/src/commands/cable.rs

- VBCABLE_URL · constant · L32-L33 — pub const VBCABLE_URL: &str =
- VBCABLE_SHA256 · constant · L38-L39 — pub const VBCABLE_SHA256: &str =
- VBCABLE_SETUP_EXE · constant · L43-L43 — const VBCABLE_SETUP_EXE: &str = "VBCABLE_Setup_x64.exe";
- RoutingSetupOutcome · enum · L51-L57 — pub enum RoutingSetupOutcome
- sha256_hex · function · L61-L70 — pub fn sha256_hex(bytes: &[u8]) -> String
- system_audio_setup_routing · function · L79-L92 — pub fn system_audio_setup_routing(app: tauri::AppHandle) -> Result<RoutingSetupOutcome, IpcError>
- setup_routing_windows · function · L95-L150 — fn setup_routing_windows(app: &tauri::AppHandle) -> Result<RoutingSetupOutcome, String>
- download · function · L154-L170 — fn download(url: &str) -> Result<Vec<u8>, String>
- path_arg · function · L176-L181 — pub(crate) fn path_arg(p: &std::path::Path) -> Result<String, String>
- powershell · function · L185-L194 — pub(crate) fn powershell(command: &str) -> Result<std::process::ExitStatus, String>
- CREATE_NO_WINDOW · constant · L188-L188 — const CREATE_NO_WINDOW: u32 = 0x0800_0000;
- expand_archive · function · L198-L209 — pub(crate) fn expand_archive(zip: &std::path::Path, dest: &std::path::Path) -> Result<(), String>
- run_elevated · function · L215-L230 — fn run_elevated(exe: &std::path::Path, args: &[&str]) -> Result<(), String>
- tests · module · L233-L262 — mod tests
- sha256_hex_empty_input · function · L238-L243 — fn sha256_hex_empty_input()
- sha256_hex_abc · function · L246-L251 — fn sha256_hex_abc()
- pinned_hash_is_lowercase_hex_sha256_shaped · function · L254-L261 — fn pinned_hash_is_lowercase_hex_sha256_shaped()
