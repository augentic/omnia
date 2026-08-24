//! Assembling a single-guest runtime for seam tests.
//!
//! [`single_guest`] collapses the deployment boilerplate every seam test
//! repeats — locate the guest, build a deployment, link hosts, assemble the
//! registry, wrap it in a [`Runtime`] — into a small chainable builder. The
//! test still declares its backend bundle (the `Has*` impls are the seam under
//! test), but nothing else.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use omnia::{
    Deployment, DeploymentBuilder, Host, Manifest, MountRegistry, Runtime, Server, StoreCtx,
    WrpcView,
};
use wasmtime_wasi::WasiView;

use crate::find_guest;

/// A single-guest deployment mid-assembly: link hosts, then build the runtime.
pub struct SingleGuest<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiView,
{
    deployment: Deployment<StoreCtx<B>>,
    bundle: B,
}

/// Start assembling a runtime over the example guest `file` backed by
/// `bundle`.
///
/// # Errors
///
/// Returns an error if the deployment cannot be built from the guest wasm.
///
/// # Panics
///
/// Panics when the guest artifact is missing (see [`find_guest`]).
// The unsafe pre-compiled build is satisfied here: `find_guest` artifacts are
// workspace-built (`cargo make test-guests`).
#[allow(unsafe_code)]
pub async fn single_guest<B>(file: &str, bundle: B) -> Result<SingleGuest<B>>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiView,
{
    let wasm = find_guest(file);
    let builder = DeploymentBuilder::new().manifest(Manifest::from_wasm(wasm)).precompiled();
    // SAFETY: `find_guest` only returns artifacts this workspace built itself
    // (`cargo make test-guests` compiles the example guests and serializes the
    // `.bin` files through omnia's own compile path).
    let deployment =
        unsafe { builder.build::<StoreCtx<B>>() }.await.context("building deployment")?;
    Ok(SingleGuest { deployment, bundle })
}

impl<B> SingleGuest<B>
where
    B: Clone + Send + Sync + 'static,
    StoreCtx<B>: WasiView,
{
    /// Link a WASI host's interfaces into the deployment. Chainable.
    ///
    /// # Errors
    ///
    /// Returns an error if the host cannot be added to the linker.
    pub fn host<H>(mut self) -> Result<Self>
    where
        H: Host<StoreCtx<B>> + Server<B>,
    {
        self.deployment.host::<H, B>()?;
        Ok(self)
    }

    /// Assemble the registry and wrap it in a [`Runtime`] over the bundle.
    ///
    /// # Errors
    ///
    /// Returns an error if the registry cannot be assembled.
    pub fn into_runtime(self) -> Result<Runtime<B>>
    where
        StoreCtx<B>: WrpcView,
    {
        let registry = self.deployment.into_registry().context("assembling registry")?;
        Ok(Runtime::from_parts(
            Arc::new(registry),
            Vec::new(),
            Arc::new(MountRegistry::default()),
            self.bundle,
        ))
    }
}
