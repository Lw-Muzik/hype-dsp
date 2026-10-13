# crates/hm-audio/src/win_driver.rs

- routing_device_available · function · L32-L34 — pub fn routing_device_available() -> bool
- find_driver_inf · function · L42-L52 — pub fn find_driver_inf(package_dir: &Path) -> Option<PathBuf>
- install_driver · function · L62-L101 — pub fn install_driver(package_dir: &Path) -> Result<(), AudioError>
- CREATE_NO_WINDOW · constant · L87-L87 — const CREATE_NO_WINDOW: u32 = 0x0800_0000;
