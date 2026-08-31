//! The canonical `sha256:<hex>` digest encoding and validation.

use std::fmt::Write as _;

use sha2::{Digest as _, Sha256};

const SCHEME: &str = "sha256:";
const HEX_LEN: usize = 64;

/// Hash `bytes` into their canonical `sha256:<hex>` digest string.
#[must_use]
pub fn sha256_digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut digest = String::with_capacity(SCHEME.len() + 2 * hash.len());
    digest.push_str(SCHEME);
    for byte in hash {
        let _ = write!(digest, "{byte:02x}");
    }
    digest
}

/// Canonicalize a digest string (`sha256:` plus 64 hex characters,
/// lowercased) so it compares byte-for-byte against
/// [`sha256_digest`](crate::sha256_digest) output.
///
/// Returns a description of the malformation when the digest is not a valid
/// digest string.
pub fn canonicalize(digest: &str) -> Result<String, String> {
    let Some(hex) = digest.strip_prefix(SCHEME) else {
        return Err(format!("digest `{digest}` is not `{SCHEME}<hex>`"));
    };
    if hex.len() != HEX_LEN || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("digest `{digest}` is not {HEX_LEN} hex characters"));
    }
    Ok(format!("{SCHEME}{}", hex.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::{canonicalize, sha256_digest};

    #[test]
    fn hash_vector() {
        // The well-known sha256 of the empty input.
        assert_eq!(
            sha256_digest(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn canonicalizes() {
        let upper = format!("sha256:{}", "AB".repeat(32));
        assert_eq!(canonicalize(&upper).expect("valid pin"), format!("sha256:{}", "ab".repeat(32)));
    }
}
