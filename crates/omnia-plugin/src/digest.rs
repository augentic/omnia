//! Validation of operator-supplied `sha256:<hex>` digest pins.

const SCHEME: &str = "sha256:";
const HEX_LEN: usize = 64;

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
    use super::canonicalize;

    #[test]
    fn canonicalizes() {
        let upper = format!("sha256:{}", "AB".repeat(32));
        assert_eq!(canonicalize(&upper).expect("valid pin"), format!("sha256:{}", "ab".repeat(32)));
    }
}
