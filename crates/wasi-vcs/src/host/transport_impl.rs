//! The `omnia:vcs/transport` host bindings.

use omnia_core::{HostView, StoreCtx};
use wasmtime::component::Accessor;

use crate::host::generated::omnia::vcs::transport::{CloneOptions, Host, HostWithStore, Location};
use crate::host::location::Intent;
use crate::host::{Result, VcsClient, WasiVcs};

impl<B: Send + 'static> HostWithStore<StoreCtx<B>> for WasiVcs {
    async fn clone(
        accessor: &Accessor<StoreCtx<B>, Self>, url: String, at: Location, options: CloneOptions,
    ) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Create)?;
            Ok(access.get().client.clone_repo(url, at, options))
        })
        .await
    }

    async fn fetch(
        accessor: &Accessor<StoreCtx<B>, Self>, repo: Location, remote: String,
    ) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Mutate)?;
            Ok(access.get().client.fetch(repo, remote))
        })
        .await
    }

    async fn label(
        accessor: &Accessor<StoreCtx<B>, Self>, repo: Location, name: String, revision: String,
    ) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Mutate)?;
            Ok(access.get().client.label(repo, name, revision))
        })
        .await
    }

    async fn labelled(
        accessor: &Accessor<StoreCtx<B>, Self>, repo: Location, name: String,
    ) -> Result<String> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Read)?;
            Ok(access.get().client.labelled(repo, name))
        })
        .await
    }

    async fn fetched(
        accessor: &Accessor<StoreCtx<B>, Self>, repo: Location, remote: String, name: String,
    ) -> Result<String> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Read)?;
            Ok(access.get().client.fetched(repo, remote, name))
        })
        .await
    }

    async fn push(
        accessor: &Accessor<StoreCtx<B>, Self>, repo: Location, remote: String, label: String,
    ) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Mutate)?;
            Ok(access.get().client.push(repo, remote, label))
        })
        .await
    }
}

impl Host for HostView<'_, dyn VcsClient> {}
