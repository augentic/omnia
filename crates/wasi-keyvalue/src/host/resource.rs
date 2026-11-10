use std::sync::Arc;

use omnia_backend::keyvalue::Bucket;

/// Proxy for a Key-Value bucket.
pub type BucketProxy = omnia_core::Proxy<dyn Bucket>;

/// CAS (Compare-And-Swap) operation handle.
#[derive(Clone, Debug)]
pub struct Cas {
    /// The bucket the operation reads from and swaps into.
    pub bucket: Arc<dyn Bucket>,

    /// The key associated with the CAS operation.
    pub key: String,

    /// The current value associated with the key.
    pub current: Option<Vec<u8>>,
}
