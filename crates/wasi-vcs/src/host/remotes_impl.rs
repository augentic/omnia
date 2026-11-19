//! The `omnia:vcs/remotes` host bindings.

use omnia_core::{HostView, StoreCtx};
use wasmtime::component::{Accessor, Resource};

use crate::host::generated::omnia::vcs::remotes::{Diverged, Host, HostWithStore};
use crate::host::generated::omnia::vcs::types::{CommitId, Error, Location, Occupied};
use crate::host::location::Intent;
use crate::host::workspace_impl::{WorkspaceHandle, found, laid};
use crate::host::{CloneOptions, Error as BackendError, VcsClient, WasiVcs};

impl<B: Send + 'static> HostWithStore<StoreCtx<B>> for WasiVcs {
    async fn clone(
        accessor: &Accessor<StoreCtx<B>, Self>, url: String, at: Location, depth: Option<u32>,
    ) -> Result<Result<Resource<WorkspaceHandle>, Occupied>, Error> {
        if depth == Some(0) {
            return Err(Error::Invalid("a clone `depth` must be at least 1".to_owned()));
        }
        let workspace = WorkspaceHandle::resolve(accessor, &at, Intent::Create)?;
        let options = CloneOptions { depth };
        let made =
            workspace.call(accessor, |client, place| client.clone_repo(url, place, options)).await;
        match laid(made)? {
            Ok(()) => Ok(Ok(workspace.push(accessor)?)),
            Err(occupied) => Ok(Err(occupied)),
        }
    }

    async fn fetch(
        accessor: &Accessor<StoreCtx<B>, Self>, workspace: Resource<WorkspaceHandle>,
        remote: String,
    ) -> Result<(), Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &workspace)?;
        workspace.writable()?;
        Ok(workspace.call(accessor, |client, place| client.fetch(place, remote)).await?)
    }

    async fn bookmark(
        accessor: &Accessor<StoreCtx<B>, Self>, workspace: Resource<WorkspaceHandle>,
        remote: String, name: String,
    ) -> Result<Option<CommitId>, Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &workspace)?;
        let answer = workspace
            .call(accessor, |client, place| client.fetched(place, remote.clone(), name.clone()))
            .await;

        // an unknown remote fails, and a bookmark the repository has not seen
        // is nothing found
        match answer.map_err(BackendError::typed) {
            Ok(commit) => Ok(Some(commit)),
            Err(BackendError::NotFound(missing)) if missing != remote || remote == name => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    async fn push(
        accessor: &Accessor<StoreCtx<B>, Self>, workspace: Resource<WorkspaceHandle>,
        remote: String, name: String,
    ) -> Result<Result<(), Diverged>, Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &workspace)?;
        workspace.writable()?;
        let pushed = workspace
            .call(accessor, |client, place| client.push(place, remote.clone(), name.clone()))
            .await;
        match pushed.map_err(BackendError::typed) {
            Ok(()) => Ok(Ok(())),
            Err(BackendError::Diverged(_)) => {
                let seen = workspace
                    .call(accessor, |client, place| client.fetched(place, remote, name))
                    .await;
                Ok(Err(Diverged {
                    current: found(seen)?,
                }))
            }
            Err(err) => Err(err.into()),
        }
    }
}

impl Host for HostView<'_, dyn VcsClient> {}
