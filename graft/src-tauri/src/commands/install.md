# src-tauri/src/commands/install.rs

- KC_SERVICE · constant · L18-L18 — const KC_SERVICE: &str = "com.hypemuzik.desktop.install";
- KC_ACCOUNT · constant · L19-L19 — const KC_ACCOUNT: &str = "install-id";
- FILE_NAME · constant · L20-L20 — const FILE_NAME: &str = "install-id";
- DEFAULT_API · constant · L23-L23 — const DEFAULT_API: &str = "http://37.60.225.220:9400/api";
- HEARTBEAT_INTERVAL · constant · L25-L25 — const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
- api_base · function · L27-L29 — fn api_base() -> String
- is_uuid · function · L31-L33 — fn is_uuid(s: &str) -> bool
- id_file · function · L35-L37 — fn id_file(app: &AppHandle) -> Option<PathBuf>
- read_file_id · function · L39-L44 — fn read_file_id(app: &AppHandle) -> Option<String>
- write_file_id · function · L46-L53 — fn write_file_id(app: &AppHandle, id: &str)
- keychain_id · function · L55-L61 — fn keychain_id() -> Option<String>
- write_keychain_id · function · L63-L67 — fn write_keychain_id(id: &str)
- resolve_install_id · function · L76-L90 — pub fn resolve_install_id(app: &AppHandle) -> String
- spawn_tracking · function · L94-L128 — pub fn spawn_tracking(app: AppHandle)
