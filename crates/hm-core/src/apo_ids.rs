//! Identity constants for the HypeMuzik APO, shared verbatim by the APO DLL
//! (`hm-apo`) and the installer (`commands/apo_setup.rs`) so registration and
//! detection can never drift. GUID generated once for this product — never
//! regenerate (a changed CLSID orphans installed registrations).

/// Our APO's class id, braced-string form (registry) — GENERATE ONCE, then frozen.
pub const CLSID_STR: &str = "{7B1C4A20-9D3E-4E8A-9F2C-11AA22BB33CC}";

/// Same id as a 128-bit constant for COM (`GUID::from_u128`).
pub const CLSID_GUID: Guid = Guid(0x7B1C4A20_9D3E_4E8A_9F2C_11AA22BB33CC);

/// Minimal platform-agnostic GUID newtype so `hm-core` needn't depend on the
/// `windows` crate; `hm-apo` converts it to `windows_core::GUID`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Guid(pub u128);
impl Guid {
    pub const fn to_u128(self) -> u128 {
        self.0
    }
}

/// Named file mapping carrying the live `EngineParamsPod` seqlock (Task 2).
pub const MAPPING_NAME: &str = "Local\\HypeMuzikApoParams";

/// `HKLM` COM registration key for the CLSID.
pub const CLSID_REGKEY: &str =
    "SOFTWARE\\Classes\\CLSID\\{7B1C4A20-9D3E-4E8A-9F2C-11AA22BB33CC}";

/// `HKLM` AudioEngine APO catalog entry.
pub const APO_REGKEY: &str =
    "SOFTWARE\\Classes\\AudioEngine\\AudioProcessingObjects\\{7B1C4A20-9D3E-4E8A-9F2C-11AA22BB33CC}";

/// The global switch that lets `audiodg.exe` load unsigned APOs.
pub const DISABLE_PROTECTED_AUDIO_DG_KEY: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Audio";
pub const DISABLE_PROTECTED_AUDIO_DG_VALUE: &str = "DisableProtectedAudioDG";

/// The endpoint FxProperties PKEY container; the APO CLSID is written into the
/// SFX/EFX (or SFX/MFX) pid slots under
/// `MMDevices\Audio\Render\{endpoint}\FxProperties`.
pub const FX_PROPERTIES_PKEY: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d}";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clsid_string_and_guid_agree() {
        // The braced string form must parse to the same 128-bit GUID the DLL uses.
        assert_eq!(CLSID_STR, "{7B1C4A20-9D3E-4E8A-9F2C-11AA22BB33CC}");
        assert_eq!(CLSID_GUID.to_u128(), 0x7B1C4A20_9D3E_4E8A_9F2C_11AA22BB33CC);
    }
    #[test]
    fn registry_paths_are_hklm_relative_and_stable() {
        assert_eq!(CLSID_REGKEY, "SOFTWARE\\Classes\\CLSID\\{7B1C4A20-9D3E-4E8A-9F2C-11AA22BB33CC}");
        assert!(APO_REGKEY.ends_with(CLSID_STR));
        assert_eq!(
            DISABLE_PROTECTED_AUDIO_DG_KEY,
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Audio"
        );
        assert_eq!(FX_PROPERTIES_PKEY, "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d}");
    }
}
