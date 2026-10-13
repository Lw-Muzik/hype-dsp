# crates/hm-core/src/license.rs

- LicenseStatus · enum · L19-L26 — pub enum LicenseStatus
- LicenseError · enum · L30-L35 — pub enum LicenseError
- LicenseService · interface · L40-L47 — pub trait LicenseService: Send + Sync
- status · function · L42-L42 — fn status(&self) -> LicenseStatus;
- activate · function · L44-L44 — fn activate(&self, key: &str) -> Result<LicenseStatus, LicenseError>;
- deactivate · function · L46-L46 — fn deactivate(&self) -> Result<(), LicenseError>;
- TRIAL_DAYS · constant · L50-L50 — pub const TRIAL_DAYS: u64 = 14;
- now_millis · function · L52-L57 — fn now_millis() -> u64
- MockState · struct · L60-L63 — struct MockState
- LicenseMock · struct · L69-L72 — pub struct LicenseMock
- open · function · L76-L89 — pub fn open(path: PathBuf) -> Self
- persist · function · L92-L97 — fn persist(path: &PathBuf, state: &MockState) -> bool
- status · function · L100-L114 — fn status(&self) -> LicenseStatus
- activate · function · L116-L126 — fn activate(&self, key: &str) -> Result<LicenseStatus, LicenseError>
- deactivate · function · L128-L135 — fn deactivate(&self) -> Result<(), LicenseError>
- tests · module · L139-L171 — mod tests
- fresh_install_is_in_trial · function · L143-L152 — fn fresh_install_is_in_trial()
- activation_flips_to_licensed_and_persists · function · L155-L170 — fn activation_flips_to_licensed_and_persists()
