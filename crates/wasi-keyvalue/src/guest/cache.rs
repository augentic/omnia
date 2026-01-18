//! Cache implementation for WASI Key-Value

use anyhow::{Context, Result};
use chrono::serde::ts_seconds;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::guest::store;
use crate::guest::store::Bucket;

/// Create a new Cache instance with the specified bucket name.
///
/// # Errors
///
/// Returns an error if there is an issue opening the bucket.
pub async fn open(bucket: &str) -> Result<Cache> {
    let bucket = store::open(bucket.to_string()).await.context("opening bucket")?;
    Ok(Cache { bucket })
}

/// A cache interface for storing and retrieving values.
#[derive(Debug)]
pub struct Cache {
    bucket: Bucket,
}

impl Cache {
    /// Get a value from the cache.
    ///
    /// # Errors
    ///
    /// Returns an error if there is an issue getting the value.
    pub async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        // retrieve entry
        let Some(entry) = self.bucket.get(key.to_string()).await.context("reading state")? else {
            return Ok(None);
        };

        // check for ttl envelope
        let Ok(ttl_val) = Cacheable::try_from(&entry) else {
            tracing::debug!("Not serialized using Cacheable");
            return Ok(Some(entry));
        };

        // check expiration
        if ttl_val.is_expired() {
            self.bucket.delete(key.to_string()).await.context("deleting expired state")?;
            return Ok(None);
        }

        Ok(Some(ttl_val.value))
    }

    /// Set a value in the cache, optionally with an expiration duration.
    /// Returns the previous value if it existed.
    ///
    /// # Errors
    ///
    /// Returns an error if there is an issue setting the value.
    pub async fn set(
        &self, key: &str, value: &[u8], ttl_secs: Option<u64>,
    ) -> Result<Option<Vec<u8>>> {
        // if TTL, create envelope
        let value = if let Some(ttl) = ttl_secs.map(|secs| Duration::seconds(secs.cast_signed())) {
            let envelope = Cacheable::new(value, ttl);
            &<Cacheable as TryInto<Vec<u8>>>::try_into(envelope)?
        } else {
            value
        };

        // return previous value
        let previous = self.get(key).await?;
        self.bucket.set(key.to_string(), value.to_vec()).await.context("setting state with ttl")?;

        Ok(previous)
    }

    /// Delete a value from the cache.
    ///
    /// # Errors
    ///
    /// Returns an error if there is an issue deleting the value.
    pub async fn delete(&self, key: &str) -> Result<()> {
        self.bucket.delete(key.to_string()).await.context("deleting entry")
    }
}

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

    #[must_use]
    pub(crate) fn is_expired(&self) -> bool {
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

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn valid() {
        let value = vec![1, 2, 3, 4];
        let expires_at = Utc::now() + Duration::seconds(60);

        let cacheable = Cacheable {
            value: value.clone(),
            expires_at,
        };

        let bytes = serde_json::to_vec(&cacheable).unwrap();
        let parsed = Cacheable::try_from(&bytes).unwrap();

        assert_eq!(parsed.value, value);
        assert_eq!(parsed.expires_at.timestamp(), expires_at.timestamp());
    }

    #[test]
    fn invalid_json() {
        let invalid = b"not a json".to_vec();
        let result = Cacheable::try_from(&invalid);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("issue deserializing Cacheable"));
    }

    #[test]
    fn wrong_type() {
        // JSON for a different type (e.g., a string)
        let bytes = serde_json::to_vec(&"just a string").unwrap();
        let result = Cacheable::try_from(&bytes);
        result.unwrap_err();
    }
}
