//! The `omnia:vcs/store` host bindings.

use omnia_core::HasMounts;
use wasmtime::component::Accessor;

use crate::host::generated::omnia::vcs::store::{Host, HostWithStore, Location, Merged, Rule};
use crate::host::location::Intent;
use crate::host::{Result, WasiVcs, WasiVcsCtxView};

impl<T> HostWithStore<T> for WasiVcs
where
    T: HasMounts,
{
    async fn resolve(
        accessor: &Accessor<T, Self>, repo: Location, revision: String,
    ) -> Result<String> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Read)?;
            Ok(access.get().ctx.resolve(repo, revision))
        })
        .await
    }

    async fn head(accessor: &Accessor<T, Self>, at: Location) -> Result<String> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Read)?;
            Ok(access.get().ctx.head(at))
        })
        .await
    }

    async fn commit(
        accessor: &Accessor<T, Self>, at: Location, message: String,
    ) -> Result<Option<String>> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Mutate)?;
            Ok(access.get().ctx.commit(at, message))
        })
        .await
    }

    async fn merge(
        accessor: &Accessor<T, Self>, at: Location, revision: String, message: String,
        policy: Vec<Rule>,
    ) -> Result<Merged> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Mutate)?;
            Ok(access.get().ctx.merge(at, revision, message, policy))
        })
        .await
    }
}

impl Host for WasiVcsCtxView<'_> {}
