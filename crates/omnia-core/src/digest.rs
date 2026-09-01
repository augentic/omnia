//! The canonical `sha256:<hex>` content-digest encoding.

use std::fmt::Write as _;

use sha2::{Digest as _, Sha256};

/// Hash `bytes` into their canonical `sha256:<hex>` digest string.
#[must_use]
pub fn sha256_digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut digest = String::with_capacity("sha256:".len() + 2 * hash.len());
    digest.push_str("sha256:");
    for byte in hash {
        let _ = write!(digest, "{byte:02x}");
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::sha256_digest;

    #[test]
    fn hash_vector() {
        // The well-known sha256 of the empty input.
        assert_eq!(
            sha256_digest(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
