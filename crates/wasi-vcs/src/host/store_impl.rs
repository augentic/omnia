//! The `omnia:vcs/store` host bindings.

use omnia_core::StoreCtx;
use wasmtime::component::Accessor;

use crate::host::generated::omnia::vcs::store::{
    Entry, Host, HostWithStore, Location, Merged, Rule,
};
use crate::host::location::Intent;
use crate::host::{Result, WasiVcs, WasiVcsCtxView};

impl<B: Send + 'static> HostWithStore<StoreCtx<B>> for WasiVcs {
    async fn resolve(
        accessor: &Accessor<StoreCtx<B>, Self>, repo: Location, revision: String,
    ) -> Result<String> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Read)?;
            Ok(access.get().ctx.resolve(repo, revision))
        })
        .await
    }

    async fn descends(
        accessor: &Accessor<StoreCtx<B>, Self>, repo: Location, ancestor: String,
        descendant: String,
    ) -> Result<bool> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Read)?;
            Ok(access.get().ctx.descends(repo, ancestor, descendant))
        })
        .await
    }

    async fn head(accessor: &Accessor<StoreCtx<B>, Self>, at: Location) -> Result<String> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Read)?;
            Ok(access.get().ctx.head(at))
        })
        .await
    }

    async fn commit(
        accessor: &Accessor<StoreCtx<B>, Self>, at: Location, message: String,
    ) -> Result<Option<String>> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Mutate)?;
            Ok(access.get().ctx.commit(at, message))
        })
        .await
    }

    async fn merge(
        accessor: &Accessor<StoreCtx<B>, Self>, at: Location, revision: String, message: String,
        policy: Vec<Rule>,
    ) -> Result<Merged> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Mutate)?;
            Ok(access.get().ctx.merge(at, revision, message, policy))
        })
        .await
    }

    async fn log(
        accessor: &Accessor<StoreCtx<B>, Self>, repo: Location, revision: String, base: String,
    ) -> Result<Vec<Entry>> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Read)?;
            Ok(access.get().ctx.log(repo, revision, base))
        })
        .await
    }
}

impl Host for WasiVcsCtxView<'_> {}
