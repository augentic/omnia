//! The `omnia:vcs/workspaces` host bindings.

use omnia_core::{HostView, StoreCtx};
use wasmtime::component::{Accessor, Resource};

use crate::generated::omnia::vcs::types::{Error, Location, Occupied};
use crate::generated::omnia::vcs::workspaces::{Host, HostWithStore};
use crate::location::Intent;
use crate::workspace_impl::{WorkspaceHandle, laid};
use crate::{VcsClient, WasiVcs};

impl<B: Send + 'static> HostWithStore<StoreCtx<B>> for WasiVcs {
    async fn add(
        accessor: &Accessor<StoreCtx<B>, Self>, workspace: Resource<WorkspaceHandle>, at: Location,
        revision: String,
    ) -> Result<Result<Resource<WorkspaceHandle>, Occupied>, Error> {
        // the repository records the workspace it adds
        let source = WorkspaceHandle::lookup(accessor, &workspace)?;
        source.writable()?;
        let added = WorkspaceHandle::resolve(accessor, &at, Intent::Create)?;

        let place = added.place().clone();
        let made = source.call(accessor, |client, repo| client.add(repo, place, revision)).await;
        match laid(made)? {
            Ok(()) => Ok(Ok(added.push(accessor)?)),
            Err(occupied) => Ok(Err(occupied)),
        }
    }

    async fn remove(
        accessor: &Accessor<StoreCtx<B>, Self>, workspace: Resource<WorkspaceHandle>,
    ) -> Result<(), Error> {
        let removed = WorkspaceHandle::lookup(accessor, &workspace)?;
        removed.writable()?;
        removed.call(accessor, |client, place| client.remove(place)).await?;
        WorkspaceHandle::mark_removed(accessor, &workspace)
    }
}

impl Host for HostView<'_, dyn VcsClient> {}
