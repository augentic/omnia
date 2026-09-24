//! The privileged runtime half the loader drives, erased of the deployment's
//! backend type.

use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_core::{AdmitError, Digest, GuestId, WeakRuntime};

use crate::error::LoadError;

/// The registry record and the admission seam — erased of the deployment's
/// backend type so [`Plugins`](crate::Plugins) can live in the runtime's
/// extensions.
pub trait Admission: Send + Sync + 'static {
    /// The registration state of `id`, with its recorded digest.
    fn registration(&self, id: &GuestId) -> Result<Registration, LoadError>;

    /// Admit component bytes, hashing to `digest`, as the late guest `id`.
    fn admit(
        &self, id: GuestId, bytes: Vec<u8>, digest: Digest,
    ) -> BoxFuture<'static, Result<(), AdmitError>>;
}

// Weak: a strong handle would cycle through the extension.
impl<B: Clone + Send + Sync + 'static> Admission for WeakRuntime<B> {
    fn registration(&self, id: &GuestId) -> Result<Registration, LoadError> {
        let runtime = self
            .upgrade()
            .ok_or_else(|| LoadError::Internal("the runtime has shut down".to_owned()))?;
        Ok(runtime
            .registry()
            .get(id)
            .map_or(Registration::Absent, |guest| Registration::Active(guest.digest())))
    }

    fn admit(
        &self, id: GuestId, bytes: Vec<u8>, digest: Digest,
    ) -> BoxFuture<'static, Result<(), AdmitError>> {
        let weak = self.clone();
        async move {
            let Some(runtime) = weak.upgrade() else {
                return Err(AdmitError::Internal("the runtime has shut down".to_owned()));
            };
            runtime.admit(id, bytes, digest).await
        }
        .boxed()
    }
}

pub enum Registration {
    Absent,
    /// Registered, with the digest the registry recorded for its bytes.
    Active(Digest),
}
