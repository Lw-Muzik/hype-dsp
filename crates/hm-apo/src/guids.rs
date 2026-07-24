//! Our APO's class id, converted from the platform-agnostic constant in
//! `hm-core` (shared verbatim with the installer) to a `windows_core::GUID`.

use windows_core::GUID;

/// The HypeMuzik APO CLSID (same 128-bit value as `hm_core::apo_ids::CLSID_GUID`).
pub const CLSID_HYPEMUZIK_APO: GUID = GUID::from_u128(hm_core::apo_ids::CLSID_GUID.to_u128());
