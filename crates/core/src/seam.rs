//! The seam between the guest registry and guest→guest linking.
//!
//! The registry owns guest lifecycle, and [`LinkSeam`] decides how a guest's
//! linked imports are satisfied. A deployment built without omnia's `link`
//! feature installs [`NoLinks`]. One built with it installs an implementation
//! whose polyfilled imports call through the calling store's
//! [`Dispatcher`](crate::Dispatcher).

use anyhow::Result;
use wasmtime::Engine;
use wasmtime::component::{Component, Linker};

use crate::registry::{Guest, GuestId};
use crate::source::LoadedGuest;
use crate::store::StoreCtx;

/// Guest→guest linking as the registry drives it, over the backend bundle `B`.
#[doc(hidden)]
pub trait LinkSeam<B: 'static>: Send + Sync + 'static {
    /// Polyfill every bootstrap guest's link imports onto the shared linker,
    /// before pre-instantiation.
    ///
    /// # Errors
    ///
    /// Returns an error if a linked import cannot be polyfilled.
    fn polyfill(
        &self, engine: &Engine, linker: &mut Linker<StoreCtx<B>>, guests: &[LoadedGuest],
    ) -> Result<()>;

    /// Polyfill a late guest's remaining link imports onto `linker`, a clone
    /// of the shared linker.
    ///
    /// # Errors
    ///
    /// Returns an error if a linked import cannot be polyfilled.
    fn polyfill_late(
        &self, engine: &Engine, linker: &mut Linker<StoreCtx<B>>, id: &GuestId,
        component: &Component,
    ) -> Result<()>;

    /// Refuse `guest` when a linked function it shares with a guest in
    /// `standing` has another signature.
    ///
    /// The registry calls it as a late guest is published, passing every
    /// guest then registered, under the guest map's write lock. It must not
    /// call back into the registry. At assembly, each boot guest is checked
    /// against the boot guests before it.
    ///
    /// # Errors
    ///
    /// Returns an error naming both guests and the function when the two
    /// signatures differ.
    fn check(&self, guest: &Guest<B>, standing: &[&Guest<B>]) -> Result<()>;
}

/// The seam of a deployment built without omnia's `link` feature.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLinks;

impl<B: 'static> LinkSeam<B> for NoLinks {
    fn polyfill(
        &self, _engine: &Engine, _linker: &mut Linker<StoreCtx<B>>, _guests: &[LoadedGuest],
    ) -> Result<()> {
        Ok(())
    }

    fn polyfill_late(
        &self, _engine: &Engine, _linker: &mut Linker<StoreCtx<B>>, _id: &GuestId,
        _component: &Component,
    ) -> Result<()> {
        Ok(())
    }

    fn check(&self, _guest: &Guest<B>, _standing: &[&Guest<B>]) -> Result<()> {
        Ok(())
    }
}
