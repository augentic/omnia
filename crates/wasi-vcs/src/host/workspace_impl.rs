//! The `omnia:vcs/workspace` host bindings.

use omnia_core::HasMounts;
use wasmtime::component::Accessor;

use crate::host::generated::omnia::vcs::workspace::{Change, Host, HostWithStore, Location};
use crate::host::location::Intent;
use crate::host::{Result, WasiVcs, WasiVcsCtxView};

impl<T> HostWithStore<T> for WasiVcs
where
    T: HasMounts,
{
    async fn init(accessor: &Accessor<T, Self>, at: Location) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Mutate)?;
            Ok(access.get().ctx.init(at))
        })
        .await
    }

    async fn add(
        accessor: &Accessor<T, Self>, repo: Location, at: Location, revision: String,
    ) -> Result<()> {
        Self::dispatch(accessor, |access| {
            // the repository records the working copy, so both locations are written
            let repo = Self::locate(access, &repo, Intent::Mutate)?;
            let at = Self::locate(access, &at, Intent::Mutate)?;
            Ok(access.get().ctx.add(repo, at, revision))
        })
        .await
    }

    async fn remove(accessor: &Accessor<T, Self>, at: Location) -> Result<()> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Mutate)?;
            Ok(access.get().ctx.remove(at))
        })
        .await
    }

    async fn pending(accessor: &Accessor<T, Self>, at: Location) -> Result<Vec<Change>> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Read)?;
            Ok(access.get().ctx.pending(at))
        })
        .await
    }
}

impl Host for WasiVcsCtxView<'_> {}
