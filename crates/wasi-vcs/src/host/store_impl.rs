//! The `omnia:vcs/store` host bindings.

use omnia_core::HasMounts;
use wasmtime::component::Accessor;

use crate::host::generated::omnia::vcs::store::{
    Entry as WitEntry, Host, HostWithStore, Location, Merged as WitMerged, Rule as WitRule,
    Strategy as WitStrategy,
};
use crate::host::{Entry, Intent, Merged, Result, Rule, Strategy, WasiVcs, WasiVcsView};

fn from_wit_rule(rule: WitRule) -> Rule {
    Rule {
        paths: rule.paths,
        strategy: match rule.strategy {
            WitStrategy::Union => Strategy::Union,
            WitStrategy::Ours => Strategy::Ours,
            WitStrategy::Theirs => Strategy::Theirs,
        },
    }
}

fn to_wit_merged(merged: Merged) -> WitMerged {
    WitMerged {
        commit: merged.commit,
        conflicts: merged.conflicts,
    }
}

fn to_wit_entry(entry: Entry) -> WitEntry {
    WitEntry {
        id: entry.id,
        message: entry.message,
    }
}

impl<T> HostWithStore<T> for WasiVcs
where
    T: HasMounts,
{
    async fn resolve(
        accessor: &Accessor<T, Self>, repo: Location, revision: String,
    ) -> Result<String> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Read)?;
            Ok(access.get().client.resolve(repo, revision))
        })
        .await
    }

    async fn descends(
        accessor: &Accessor<T, Self>, repo: Location, ancestor: String, descendant: String,
    ) -> Result<bool> {
        Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Read)?;
            Ok(access.get().client.descends(repo, ancestor, descendant))
        })
        .await
    }

    async fn head(accessor: &Accessor<T, Self>, at: Location) -> Result<String> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Read)?;
            Ok(access.get().client.head(at))
        })
        .await
    }

    async fn commit(
        accessor: &Accessor<T, Self>, at: Location, message: String,
    ) -> Result<Option<String>> {
        Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Mutate)?;
            Ok(access.get().client.commit(at, message))
        })
        .await
    }

    async fn merge(
        accessor: &Accessor<T, Self>, at: Location, revision: String, message: String,
        policy: Vec<WitRule>,
    ) -> Result<WitMerged> {
        let policy = policy.into_iter().map(from_wit_rule).collect();
        let merged = Self::dispatch(accessor, |access| {
            let at = Self::locate(access, &at, Intent::Mutate)?;
            Ok(access.get().client.merge(at, revision, message, policy))
        })
        .await?;
        Ok(to_wit_merged(merged))
    }

    async fn log(
        accessor: &Accessor<T, Self>, repo: Location, revision: String, base: String,
    ) -> Result<Vec<WitEntry>> {
        let entries = Self::dispatch(accessor, |access| {
            let repo = Self::locate(access, &repo, Intent::Read)?;
            Ok(access.get().client.log(repo, revision, base))
        })
        .await?;
        Ok(entries.into_iter().map(to_wit_entry).collect())
    }
}

impl Host for WasiVcsView<'_> {}
