//! The `omnia:vcs/store` host bindings.

use omnia_core::{HostView, StoreCtx};
use wasmtime::component::{Accessor, Resource};

use crate::host::generated::omnia::vcs::store::{Host, HostWithStore};
use crate::host::generated::omnia::vcs::types::{Error, Location, Occupied};
use crate::host::location::Intent;
use crate::host::workspace_impl::{WorkspaceHandle, found, laid};
use crate::host::{VcsClient, WasiVcs};

impl<B: Send + 'static> HostWithStore<StoreCtx<B>> for WasiVcs {
    async fn open(
        accessor: &Accessor<StoreCtx<B>, Self>, at: Location,
    ) -> Result<Resource<WorkspaceHandle>, Error> {
        let workspace = WorkspaceHandle::resolve(accessor, &at, Intent::Open)?;

        // a workspace with no commits has no head, but is still a workspace
        found(workspace.call(accessor, |client, place| client.head(place)).await)?;
        workspace.push(accessor)
    }

    async fn init(
        accessor: &Accessor<StoreCtx<B>, Self>, at: Location,
    ) -> Result<Result<Resource<WorkspaceHandle>, Occupied>, Error> {
        let workspace = WorkspaceHandle::resolve(accessor, &at, Intent::Create)?;
        let made = workspace.call(accessor, |client, place| client.init(place)).await;
        match laid(made)? {
            Ok(()) => Ok(Ok(workspace.push(accessor)?)),
            Err(occupied) => Ok(Err(occupied)),
        }
    }
}

impl Host for HostView<'_, dyn VcsClient> {}
