//! Persistent iroh identity — so this endpoint keeps a *stable* id across runs
//! (a phone that paired once can be redialed by id forever).

use anyhow::{anyhow, Result};
use iroh::SecretKey;
use std::path::Path;

/// Load the persisted 32-byte secret, or generate + save a new one.
pub fn load_or_create_secret(path: &Path) -> Result<SecretKey> {
    if let Ok(bytes) = std::fs::read(path) {
        if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
            return Ok(SecretKey::from_bytes(&arr));
        }
    }
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| anyhow!("rng failed: {e}"))?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, seed)?;
    Ok(SecretKey::from_bytes(&seed))
}

/// Six random decimal digits, for a pairing PIN.
pub fn random_pin() -> String {
    let mut bytes = [0u8; 3];
    let _ = getrandom::getrandom(&mut bytes);
    let n = (u32::from(bytes[0]) << 16 | u32::from(bytes[1]) << 8 | u32::from(bytes[2])) % 1_000_000;
    format!("{n:06}")
}
