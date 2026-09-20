//! Reading state back through the bundle's host handles after a run.

use omnia_wasi_blobstore::WasiBlobstoreCtx;
use omnia_wasi_keyvalue::WasiKeyValueCtx;

use super::{Backends, STATE_BUCKET};

impl<M, K, B, D, S, V, G, I, O> Backends<M, K, B, D, S, V, G, I, O>
where
    K: WasiKeyValueCtx,
    B: WasiBlobstoreCtx,
    // `&self` is held across the awaits below, so the rest of the bundle must
    // be `Sync` for the futures to be `Send`; every `Wasi*Ctx` already is.
    M: Sync,
    D: Sync,
    S: Sync,
    V: Sync,
    G: Sync,
    I: Sync,
    O: Sync,
{
    /// One entry of the bucket the guest's `StateStore` writes, read through
    /// the keyvalue handle.
    pub async fn state(&self, key: &str) -> Option<Vec<u8>> {
        let bucket = self.keyvalue.open_bucket(STATE_BUCKET.to_owned()).await.ok()?;
        bucket.get(key.to_owned()).await.ok().flatten()
    }

    /// One committed object, read through the blobstore handle.
    pub async fn object(&self, container: &str, name: &str) -> Option<Vec<u8>> {
        if !self.blobstore.container_exists(container.to_owned()).await.ok()? {
            return None;
        }
        let container = self.blobstore.get_container(container.to_owned()).await.ok()?;
        container.get_data(name.to_owned(), 0, u64::MAX).await.ok().flatten().map(Into::into)
    }
}
