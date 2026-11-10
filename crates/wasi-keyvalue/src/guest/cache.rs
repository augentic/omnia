//! Cache implementation for WASI Key-Value

use anyhow::{Context, Result};
use chrono::Duration;

use crate::Cacheable;
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
        let Some(entry) = self.bucket.get(key.to_string()).await.context("reading state")? else {
            return Ok(None);
        };

        // raw bytes another writer stored are the normal case for a shared bucket
        let Ok(ttl_val) = Cacheable::try_from(&entry) else {
            tracing::trace!("Not serialized using Cacheable");
            return Ok(Some(entry));
        };

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
        let stored = if let Some(ttl) = ttl_secs.map(|secs| Duration::seconds(secs.cast_signed())) {
            Cacheable::new(value, ttl).try_into()?
        } else {
            value.to_vec()
        };

        let previous = self.get(key).await?;
        self.bucket.set(key.to_string(), stored).await.context("setting state with ttl")?;

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
