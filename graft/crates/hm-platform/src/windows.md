# crates/hm-platform/src/windows.rs

- SessionInfo · struct · L38-L44 — struct SessionInfo
- process_image_path · function · L47-L59 — unsafe fn process_image_path(pid: u32) -> Option<String>
- basename · function · L61-L63 — fn basename(path: &str) -> &str
- display_name · function · L65-L71 — fn display_name(exe: &str) -> String
- session_enumerator · function · L75-L82 — unsafe fn session_enumerator() -> windows::core::Result<IAudioSessionEnumerator>
- collect_sessions · function · L85-L123 — unsafe fn collect_sessions() -> windows::core::Result<Vec<SessionInfo>>
- apply · function · L126-L150 — unsafe fn apply(
- WindowsSessionController · struct · L153-L153 — pub struct WindowsSessionController;
- new · function · L156-L158 — pub fn new() -> Self
- supported · function · L162-L164 — fn supported(&self) -> bool
- unavailable_reason · function · L166-L168 — fn unavailable_reason(&self) -> Option<String>
- list_sessions · function · L170-L184 — fn list_sessions(&self) -> Vec<AppSession>
- set_volume · function · L186-L192 — fn set_volume(&self, id: &str, gain: f32) -> Result<(), PlatformError>
- set_muted · function · L194-L200 — fn set_muted(&self, id: &str, muted: bool) -> Result<(), PlatformError>
