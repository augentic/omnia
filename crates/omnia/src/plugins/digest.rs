//! sha256 content digests: the operator pin format and byte hashing.

use sha2::{Digest as _, Sha256};

/// The canonical digest scheme prefix.
const SCHEME: &str = "sha256:";

/// Hex characters in a sha256 digest.
const HEX_LEN: usize = 64;

/// Hash `bytes` into the canonical `sha256:<hex>` digest string.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut digest = String::with_capacity(SCHEME.len() + HEX_LEN);
    digest.push_str(SCHEME);
    for byte in hash {
        digest.push(nibble(byte >> 4));
        digest.push(nibble(byte & 0xf));
    }
    digest
}

fn nibble(value: u8) -> char {
    char::from_digit(u32::from(value), 16).expect("a nibble is a hex digit")
}

/// Canonicalize an operator digest pin (`sha256:` plus 64 hex characters,
/// lowercased) so pins compare byte-for-byte against [`sha256_hex`] output.
///
/// Returns a description of the malformation when the pin is not a sha256
/// digest string.
pub fn canonicalize_pin(pin: &str) -> Result<String, String> {
    let Some(hex) = pin.strip_prefix(SCHEME) else {
        return Err(format!("digest pin `{pin}` does not use the `sha256:<hex>` scheme"));
    };
    if hex.len() != HEX_LEN || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("digest pin `{pin}` is not {HEX_LEN} hex characters"));
    }
    Ok(format!("{SCHEME}{}", hex.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_pin, sha256_hex};

    #[test]
    fn hash_known_vector() {
        // The well-known sha256 of the empty input.
        assert_eq!(
            sha256_hex(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn pin_canonicalizes_case() {
        let upper = format!("sha256:{}", "AB".repeat(32));
        assert_eq!(
            canonicalize_pin(&upper).expect("valid pin"),
            format!("sha256:{}", "ab".repeat(32))
        );
    }

    #[test]
    fn pin_rejects_wrong_scheme() {
        let error =
            canonicalize_pin(&format!("sha512:{}", "ab".repeat(32))).expect_err("scheme refused");
        assert!(error.contains("sha256:<hex>"));
    }

    #[test]
    fn pin_rejects_wrong_length_and_non_hex() {
        assert!(canonicalize_pin("sha256:abcd").is_err(), "short hex refused");
        let bad = format!("sha256:{}", "zz".repeat(32));
        assert!(canonicalize_pin(&bad).is_err(), "non-hex refused");
    }
}
