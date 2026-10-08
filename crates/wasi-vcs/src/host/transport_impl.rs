//! The `omnia:vcs/transport` host bindings.

use omnia_core::HasMounts;
use wasmtime::component::Accessor;

use crate::host::generated::omnia::vcs::transport::{CloneOptions, Host, HostWithStore, Location};
use crate::host::location::Intent;
use crate::host::{Result, WasiVcs, WasiVcsCtxView};

impl<T> HostWithStore<T> for WasiVcs
where
    T: HasMounts,
{
    async fn clone(
        accessor: &Accessor<T, Self>, url: String, at: Location, options: CloneOptions,
    ) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Mutate)?;
            Ok(access.get().ctx.clone_repo(url, at, options))
        })
        .await
    }

    async fn fetch(accessor: &Accessor<T, Self>, repo: Location, remote: String) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Mutate)?;
            Ok(access.get().ctx.fetch(repo, remote))
        })
        .await
    }

    async fn label(
        accessor: &Accessor<T, Self>, repo: Location, name: String, revision: String,
    ) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Mutate)?;
            Ok(access.get().ctx.label(repo, name, revision))
        })
        .await
    }

    async fn push(
        accessor: &Accessor<T, Self>, repo: Location, remote: String, label: String,
    ) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Mutate)?;
            Ok(access.get().ctx.push(repo, remote, label))
        })
        .await
    }
}

impl Host for WasiVcsCtxView<'_> {}
