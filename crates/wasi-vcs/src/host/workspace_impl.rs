//! The `omnia:vcs/workspace` host bindings.

use omnia_core::{HostView, StoreCtx};
use wasmtime::component::Accessor;

use crate::host::generated::omnia::vcs::workspace::{Change, Host, HostWithStore, Location};
use crate::host::location::Intent;
use crate::host::{Result, WasiVcs, WasiVcsCtx};

impl<B: Send + 'static> HostWithStore<StoreCtx<B>> for WasiVcs {
    async fn init(accessor: &Accessor<StoreCtx<B>, Self>, at: Location) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Create)?;
            Ok(access.get().client.init(at))
        })
        .await
    }

    async fn add(
        accessor: &Accessor<StoreCtx<B>, Self>, repo: Location, at: Location, revision: String,
    ) -> Result<()> {
        Self::dispatch(accessor, |access| {
            // the repository records the working copy, so both locations are written
            let repo = Self::locate(access, &repo, Intent::Mutate)?;
            let at = Self::locate(access, &at, Intent::Create)?;
            Ok(access.get().client.add(repo, at, revision))
        })
        .await
    }

    async fn remove(accessor: &Accessor<StoreCtx<B>, Self>, at: Location) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Mutate)?;
            Ok(access.get().client.remove(at))
        })
        .await
    }

    async fn pending(accessor: &Accessor<StoreCtx<B>, Self>, at: Location) -> Result<Vec<Change>> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Read)?;
            Ok(access.get().client.pending(at))
        })
        .await
    }
}

impl Host for HostView<'_, dyn WasiVcsCtx> {}
