//! The TTL envelope a guest cache stores a value set with an expiry in.

use anyhow::{Context, Result};
use chrono::serde::ts_seconds;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// A type that allows for transfer of value types between guest and host where
/// the implementation may be able to manage value lifetime for an individual
/// key.
///
/// If the underlying store does not support a key-level TTL, the timestamp
/// could be used by a guest to implement expiration behavior.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Cacheable {
    /// The value to cache, in bytes.
    pub value: Vec<u8>,

    /// Time the cached value expires at.
    #[serde(with = "ts_seconds")]
    pub expires_at: DateTime<Utc>,
}

impl Cacheable {
    /// Create a new Cacheable with a value and duration until expiration.
    #[must_use]
    pub fn new(value: &[u8], expires_in: Duration) -> Self {
        Self {
            value: value.to_vec(),
            expires_at: Utc::now() + expires_in,
        }
    }

    /// Whether the value's expiry time has passed.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }
}

impl TryFrom<&Vec<u8>> for Cacheable {
    type Error = anyhow::Error;

    fn try_from(value: &Vec<u8>) -> Result<Self, Self::Error> {
        serde_json::from_slice(value).context("issue deserializing Cacheable")
    }
}

impl TryFrom<Cacheable> for Vec<u8> {
    type Error = anyhow::Error;

    fn try_from(value: Cacheable) -> Result<Self, Self::Error> {
        serde_json::to_vec(&value).context("issue serializing Cacheable")
    }
}
