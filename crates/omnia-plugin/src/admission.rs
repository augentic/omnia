//! The privileged runtime half the loader drives, erased of the deployment's
//! backend type.

use futures::FutureExt as _;
use futures::future::BoxFuture;
use omnia_core::{Digest, GuestError, GuestId, Policy, SourceSpec, WeakRuntime};

use crate::loader::Plugin;

/// The runtime's first-use, acquisition, and admission seams — erased of the
/// deployment's backend type so [`Plugins`](crate::Plugins) can live in the
/// runtime's extensions.
pub trait Admission: Send + Sync + 'static {
    /// Whether caller-named bytes may seat as `id` at all, asked before any
    /// are acquired; a name the deployment declares is refused.
    fn admits(&self, id: &GuestId) -> Result<(), GuestError>;

    /// Ensure the guest the deployment declares as `id` is loaded, and
    /// return its handle.
    fn guest(&self, id: &GuestId) -> BoxFuture<'static, Result<Plugin, GuestError>>;

    /// The bytes the exact `package` reference names, fetched through the
    /// runtime's registry source — from `endpoint` when the deployment's
    /// routing leaves the namespace open.
    fn acquire(
        &self, package: &str, endpoint: Option<&str>,
    ) -> BoxFuture<'static, Result<Vec<u8>, GuestError>>;

    /// Admit caller-named `bytes` as `id`, held to `pin`, and return the
    /// handle of the guest standing under `id` afterwards.
    fn admit_bytes(
        &self, id: GuestId, bytes: Vec<u8>, pin: Option<Digest>,
    ) -> BoxFuture<'static, Result<Plugin, GuestError>>;
}

// Weak: a strong handle would cycle through the extension.
impl<B: Clone + Send + Sync + 'static> Admission for WeakRuntime<B> {
    fn admits(&self, id: &GuestId) -> Result<(), GuestError> {
        live(self)?.admits(id, &Policy::CallerNamed { pin: None })
    }

    fn guest(&self, id: &GuestId) -> BoxFuture<'static, Result<Plugin, GuestError>> {
        let weak = self.clone();
        let id = id.clone();
        async move { live(&weak)?.guest(&id).await.map(|guest| Plugin::of(&guest)) }.boxed()
    }

    fn acquire(
        &self, package: &str, endpoint: Option<&str>,
    ) -> BoxFuture<'static, Result<Vec<u8>, GuestError>> {
        let weak = self.clone();
        let spec = SourceSpec::package(package);
        let endpoint = endpoint.map(str::to_owned);
        async move { live(&weak)?.acquire(&spec, endpoint.as_deref()).await }.boxed()
    }

    fn admit_bytes(
        &self, id: GuestId, bytes: Vec<u8>, pin: Option<Digest>,
    ) -> BoxFuture<'static, Result<Plugin, GuestError>> {
        let weak = self.clone();
        async move {
            let runtime = live(&weak)?;
            let guest = runtime.admit_bytes(id, bytes, Policy::CallerNamed { pin }).await?;
            Ok(Plugin::of(&guest))
        }
        .boxed()
    }
}

fn live<B: Clone + Send + Sync + 'static>(
    weak: &WeakRuntime<B>,
) -> Result<omnia_core::Runtime<B>, GuestError> {
    weak.upgrade().ok_or_else(|| GuestError::Internal("the runtime has shut down".to_owned()))
}
