//! The canonical `sha256:<hex>` content digest.

use std::fmt;
use std::str::FromStr;

use anyhow::{Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

const SCHEME: &str = "sha256:";
const HEX_LEN: usize = 64;

/// A sha256 content digest, spelled `sha256:<64 hex characters>`.
///
/// The pin a deployment declares for a guest's bytes, and the record a
/// registered guest carries of the bytes it was admitted from. Parsing
/// canonicalizes the hex case, so two spellings of one digest compare equal.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct Digest([u8; 32]);

impl Digest {
    /// The digest of `bytes`.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// The digest of `bytes`, held to `pin` when one is given; `subject`
    /// names the bytes in the refusal.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes hash to anything but `pin`.
    pub fn checked(bytes: &[u8], pin: Option<Self>, subject: impl fmt::Display) -> Result<Self> {
        let digest = Self::of(bytes);
        if let Some(pin) = pin {
            ensure!(pin == digest, "{subject} resolved to {digest}, not its declared digest {pin}");
        }
        Ok(digest)
    }
}

impl From<[u8; 32]> for Digest {
    fn from(hash: [u8; 32]) -> Self {
        Self(hash)
    }
}

impl FromStr for Digest {
    type Err = anyhow::Error;

    fn from_str(digest: &str) -> Result<Self> {
        let Some(hex) = digest.strip_prefix(SCHEME) else {
            bail!("digest `{digest}` is not `{SCHEME}<hex>`");
        };
        ensure!(hex.len() == HEX_LEN, "digest `{digest}` is not {HEX_LEN} hex characters");
        let mut hash = [0u8; 32];
        let (pairs, _) = hex.as_bytes().as_chunks::<2>();
        for (byte, &[high, low]) in hash.iter_mut().zip(pairs) {
            let (Some(high), Some(low)) = (nibble(high), nibble(low)) else {
                bail!("digest `{digest}` is not {HEX_LEN} hex characters");
            };
            *byte = (high << 4) | low;
        }
        Ok(Self(hash))
    }
}

const fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

impl TryFrom<String> for Digest {
    type Error = anyhow::Error;

    fn try_from(digest: String) -> Result<Self> {
        digest.parse()
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(SCHEME)?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

// Logs and assertion failures read the canonical spelling, not a byte array.
impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::Digest;

    const EMPTY: &str = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn hash_vector() {
        // the well-known sha256 of the empty input
        assert_eq!(Digest::of(b"").to_string(), EMPTY);
    }

    #[test]
    fn parse_canonicalizes_case() {
        let parsed: Digest =
            EMPTY.to_ascii_uppercase().replace("SHA256", "sha256").parse().expect("uppercase hex");
        assert_eq!(parsed, Digest::of(b""));
        assert_eq!(parsed.to_string(), EMPTY, "the spelling is lowercase whatever was parsed");
    }

    #[test]
    fn malformed() {
        for digest in [
            format!("sha512:{}", "ab".repeat(32)),
            "sha256:abcd".to_owned(),
            format!("sha256:{}", "zz".repeat(32)),
            format!("sha256:{}", "é".repeat(32)),
        ] {
            digest.parse::<Digest>().expect_err("refused");
        }
    }
}
