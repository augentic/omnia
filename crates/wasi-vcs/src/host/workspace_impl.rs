//! The `omnia:vcs/workspace` host bindings.

use omnia_core::{HostView, StoreCtx};
use wasmtime::component::Accessor;

use crate::host::generated::omnia::vcs::workspace::{
    Change as WitChange, ChangeKind as WitChangeKind, Host, HostWithStore, Location,
};
use crate::host::{Change, ChangeKind, Intent, Result, VcsClient, WasiVcs};

fn to_wit_change(change: Change) -> WitChange {
    WitChange {
        path: change.path,
        kind: match change.kind {
            ChangeKind::Added => WitChangeKind::Added,
            ChangeKind::Modified => WitChangeKind::Modified,
            ChangeKind::Deleted => WitChangeKind::Deleted,
        },
    }
}

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

    async fn pending(
        accessor: &Accessor<StoreCtx<B>, Self>, at: Location,
    ) -> Result<Vec<WitChange>> {
        let changes = Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Read)?;
            Ok(access.get().client.pending(at))
        })
        .await?;
        Ok(changes.into_iter().map(to_wit_change).collect())
    }
}

impl Host for HostView<'_, dyn VcsClient> {}
