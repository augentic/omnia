//! Operator sha256 digest pins.

/// The canonical digest scheme prefix.
const SCHEME: &str = "sha256:";

/// Hex characters in a sha256 digest.
const HEX_LEN: usize = 64;

/// Canonicalize an operator digest pin (`sha256:` plus 64 hex characters,
/// lowercased) so pins compare byte-for-byte against
/// [`omnia_plugin::sha256_digest`] output.
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
    use super::canonicalize_pin;

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
