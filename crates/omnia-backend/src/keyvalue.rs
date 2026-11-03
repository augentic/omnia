//! Key-value storage service contract and constants.

#[cfg(not(target_arch = "wasm32"))]
mod client;

use anyhow::{Context as _, Result, anyhow};

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::{Bucket, KeyValueClient};

/// The bucket `StateStore` opens.
pub const BUCKET: &str = "cache";

/// Add `delta` to an 8-byte big-endian signed integer (`None` starts at zero).
///
/// # Errors
///
/// Returns an error if `current` is `Some` and not 8 bytes, or if adding
/// `delta` overflows `i64`.
pub fn add_i64(current: Option<&[u8]>, delta: i64) -> Result<i64> {
    let base = match current {
        None => 0,
        Some(value) => {
            let bytes: [u8; 8] = value.try_into().map_err(|_len| {
                anyhow!("value is {} bytes, not an 8-byte big-endian integer", value.len())
            })?;
            i64::from_be_bytes(bytes)
        }
    };
    base.checked_add(delta).context("adding delta overflows i64")
}
