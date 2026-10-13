# src-tauri/src/commands/apo_setup.rs

- ApoSetupOutcome · enum · L16-L19 — pub enum ApoSetupOutcome
- apo_setup · function · L24-L37 — pub fn apo_setup(app: tauri::AppHandle) -> Result<ApoSetupOutcome, IpcError>
- apo_uninstall · function · L41-L51 — pub fn apo_uninstall(app: tauri::AppHandle) -> Result<(), IpcError>
- apo_repair · function · L57-L66 — pub fn apo_repair() -> Result<(), IpcError>
- imp · module · L69-L131 — mod imp
- CREATE_NO_WINDOW · constant · L79-L79 — const CREATE_NO_WINDOW: u32 = 0x0800_0000;
- setup · function · L81-L100 — pub fn setup(app: &tauri::AppHandle) -> Result<ApoSetupOutcome, String>
- uninstall · function · L102-L104 — pub fn uninstall(_app: &tauri::AppHandle) -> Result<(), String>
- run_elevated · function · L108-L130 — fn run_elevated(args: &[&str]) -> Result<(), String>
