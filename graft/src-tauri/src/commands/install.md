# src-tauri/src/commands/install.rs

- KC_SERVICE · constant · L18-L18 — const KC_SERVICE: &str = "com.hypemuzik.desktop.install";
- KC_ACCOUNT · constant · L19-L19 — const KC_ACCOUNT: &str = "install-id";
- FILE_NAME · constant · L20-L20 — const FILE_NAME: &str = "install-id";
- PLATFORM · constant · L26-L26 — const PLATFORM: &str = "desktop";
- HEARTBEAT_INTERVAL · constant · L27-L27 — const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
- api_base · function · L29-L31 — pub(crate) fn api_base() -> String
- is_uuid · function · L33-L35 — fn is_uuid(s: &str) -> bool
- id_file · function · L37-L39 — fn id_file(app: &AppHandle) -> Option<PathBuf>
- read_file_id · function · L41-L46 — fn read_file_id(app: &AppHandle) -> Option<String>
- write_file_id · function · L48-L55 — fn write_file_id(app: &AppHandle, id: &str)
- kc_service · function · L60-L65 — fn kc_service() -> String
- keychain_id · function · L67-L73 — fn keychain_id() -> Option<String>
- write_keychain_id · function · L75-L79 — fn write_keychain_id(id: &str)
- resolve_install_id · function · L88-L102 — pub fn resolve_install_id(app: &AppHandle) -> String
- spawn_tracking · function · L106-L141 — pub fn spawn_tracking(app: AppHandle)
