//! The `omnia:vcs/workspace` host bindings, and the handle every interface
//! opens.

use std::sync::Arc;

use omnia_core::{Grants, HostView, StoreCtx};
use wasmtime::component::{Access, Accessor, Resource};

use crate::generated::omnia::vcs::types::{
    CommitId, Conflict, Error, Location, Occupied, Precondition,
};
use crate::generated::omnia::vcs::workspace::{
    self, Host, HostWorkspace, HostWorkspaceWithStore, LogPage, StatusPage, Unmerged,
};
use crate::location::Intent;
use crate::{
    Change, ChangeKind, Entry, Error as BackendError, FutureResult, Merged, Place, Rule, Strategy,
    VcsClient, WasiVcs,
};

// The most entries a page holds, whatever `limit` asks for.
const MAX_PAGE_SIZE: u32 = 1000;

// The host's side of a guest's `workspace` handle: the place it opened.
// Every call on a handle `remove` marked fails.
#[derive(Clone, Debug)]
pub struct WorkspaceHandle {
    place: Place,
    removed: bool,
}

impl WorkspaceHandle {
    // Resolve `location` inside the store access, where its borrowed root is
    // valid. The backend then works through the place, never through a path
    // a guest could redirect meanwhile.
    pub fn resolve<B>(
        accessor: &Accessor<StoreCtx<B>, WasiVcs>, location: &Location, intent: Intent,
    ) -> Result<Self, Error> {
        accessor.with(|mut access| {
            let mounts = access.data_mut().mounts();
            let descriptor = access.get().table.get(&location.root)?;
            let place = Place::resolve(descriptor, &mounts, location, intent)?;
            Ok(Self {
                place,
                removed: false,
            })
        })
    }

    pub fn push<T>(self, accessor: &Accessor<T, WasiVcs>) -> Result<Resource<Self>, Error> {
        Ok(accessor.with(|mut store| store.get().table.push(self))?)
    }

    pub fn lookup<T>(
        accessor: &Accessor<T, WasiVcs>, resource: &Resource<Self>,
    ) -> Result<Self, Error> {
        let handle = omnia_core::get_cloned(accessor, resource)?;
        if handle.removed {
            return Err(Error::NotFound("the workspace was removed".to_owned()));
        }
        Ok(handle)
    }

    pub fn mark_removed<T>(
        accessor: &Accessor<T, WasiVcs>, resource: &Resource<Self>,
    ) -> Result<(), Error> {
        accessor.with(|mut store| {
            store.get().table.get_mut(resource)?.removed = true;
            Ok(())
        })
    }

    pub const fn place(&self) -> &Place {
        &self.place
    }

    // refuses a change beneath a read-only mount before any backend runs
    pub fn writable(&self) -> Result<(), Error> {
        if self.place.writable() {
            Ok(())
        } else {
            Err(Error::Denied("the workspace is beneath a read-only mount".to_owned()))
        }
    }

    // runs `call` on the deployment's backend, at this workspace's place, and
    // types the failure it returns
    pub async fn call<T, R>(
        &self, accessor: &Accessor<T, WasiVcs>,
        call: impl FnOnce(&dyn VcsClient, Place) -> FutureResult<R>,
    ) -> Result<R, BackendError> {
        let answer = accessor.with(|mut store| call(store.get().client, self.place.clone()));
        answer.await.map_err(BackendError::typed)
    }

    // as `call`, handing the backend the grants of the store's guest to hold
    // what the call dials
    pub async fn dial<B, R>(
        &self, accessor: &Accessor<StoreCtx<B>, WasiVcs>,
        call: impl FnOnce(&dyn VcsClient, Place, Arc<Grants>) -> FutureResult<R>,
    ) -> Result<R, BackendError> {
        let answer = accessor.with(|mut store| {
            let grants = store.data_mut().grants();
            call(store.get().client, self.place.clone(), grants)
        });
        answer.await.map_err(BackendError::typed)
    }
}

impl<T> HostWorkspaceWithStore<T> for WasiVcs {
    async fn resolve(
        accessor: &Accessor<T, Self>, self_: Resource<WorkspaceHandle>, revision: String,
    ) -> Result<Option<CommitId>, Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &self_)?;
        found(workspace.call(accessor, |client, place| client.resolve(place, revision)).await)
    }

    async fn is_ancestor(
        accessor: &Accessor<T, Self>, self_: Resource<WorkspaceHandle>, ancestor: String,
        descendant: String,
    ) -> Result<bool, Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &self_)?;
        let answer =
            workspace.call(accessor, |client, place| client.descends(place, ancestor, descendant));
        Ok(answer.await?)
    }

    async fn log(
        accessor: &Accessor<T, Self>, self_: Resource<WorkspaceHandle>, revision: String,
        base: Option<String>, after: Option<String>, limit: Option<u32>,
    ) -> Result<LogPage, Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &self_)?;
        let limit = page_size(limit)?;
        let Some(base) = base else {
            return Err(Error::Unsupported("this backend logs back to a base only".to_owned()));
        };

        // a page resumes at the first entry the previous one left out
        let start = after.unwrap_or(revision);
        let mut entries =
            workspace.call(accessor, |client, place| client.log(place, start, base)).await?;
        let next = entries.get(limit).map(|entry| entry.id.clone());
        entries.truncate(limit);
        Ok(LogPage {
            entries: entries.into_iter().map(Into::into).collect(),
            next,
        })
    }

    async fn head(
        accessor: &Accessor<T, Self>, self_: Resource<WorkspaceHandle>,
    ) -> Result<Option<CommitId>, Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &self_)?;
        found(workspace.call(accessor, |client, place| client.head(place)).await)
    }

    async fn status(
        accessor: &Accessor<T, Self>, self_: Resource<WorkspaceHandle>, after: Option<String>,
        limit: Option<u32>,
    ) -> Result<StatusPage, Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &self_)?;
        let limit = page_size(limit)?;
        let mut changes = workspace.call(accessor, |client, place| client.pending(place)).await?;

        // a page resumes after the last path the previous one held
        changes.sort_by(|left, right| left.path.cmp(&right.path));
        if let Some(after) = after {
            changes.retain(|change| change.path > after);
        }
        let next = (changes.len() > limit).then(|| changes[limit - 1].path.clone());
        changes.truncate(limit);
        Ok(StatusPage {
            changes: changes.into_iter().map(Into::into).collect(),
            next,
        })
    }

    async fn commit(
        accessor: &Accessor<T, Self>, self_: Resource<WorkspaceHandle>, message: String,
    ) -> Result<Option<CommitId>, Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &self_)?;
        workspace.writable()?;
        Ok(workspace.call(accessor, |client, place| client.commit(place, message)).await?)
    }

    async fn merge(
        accessor: &Accessor<T, Self>, self_: Resource<WorkspaceHandle>, revision: String,
        message: String, policy: Vec<workspace::Rule>,
    ) -> Result<Result<CommitId, Unmerged>, Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &self_)?;
        workspace.writable()?;

        // only a working copy with nothing pending takes a merge
        let pending = workspace.call(accessor, |client, place| client.pending(place)).await?;
        if !pending.is_empty() {
            let paths = pending.into_iter().map(|change| change.path).collect();
            return Ok(Err(Unmerged::Pending(sorted(paths))));
        }

        let policy = policy.into_iter().map(Into::into).collect();
        let merged = workspace
            .call(accessor, |client, place| client.merge(place, revision, message, policy))
            .await;
        match merged {
            Ok(Merged {
                commit: Some(commit), ..
            }) => Ok(Ok(commit)),
            Ok(Merged {
                commit: None,
                conflicts,
            }) => Ok(Err(Unmerged::Conflicts(sorted(conflicts)))),
            Err(BackendError::Pending(paths)) => Ok(Err(Unmerged::Pending(sorted(paths)))),
            Err(err) => Err(err.into()),
        }
    }

    async fn bookmark(
        accessor: &Accessor<T, Self>, self_: Resource<WorkspaceHandle>, name: String,
    ) -> Result<Option<CommitId>, Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &self_)?;
        found(workspace.call(accessor, |client, place| client.labelled(place, name)).await)
    }

    async fn set_bookmark(
        accessor: &Accessor<T, Self>, self_: Resource<WorkspaceHandle>, name: String,
        revision: String, when: Option<Precondition>,
    ) -> Result<Result<(), Conflict>, Error> {
        let workspace = WorkspaceHandle::lookup(accessor, &self_)?;
        if when.is_some() {
            return Err(Error::Unsupported(
                "this backend cannot check a precondition before it moves a bookmark".to_owned(),
            ));
        }
        workspace.writable()?;
        workspace.call(accessor, |client, place| client.label(place, name, revision)).await?;
        Ok(Ok(()))
    }

    fn drop(mut host: Access<'_, T, Self>, rep: Resource<WorkspaceHandle>) -> wasmtime::Result<()> {
        host.get().table.delete(rep)?;
        Ok(())
    }
}

// What a lookup finds. A backend's `NotFound` is nothing found.
pub fn found<R>(answer: Result<R, BackendError>) -> Result<Option<R>, Error> {
    match answer {
        Ok(value) => Ok(Some(value)),
        Err(BackendError::NotFound(_)) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

// What a call that lays a workspace down did. A backend's `Exists` is the
// place found occupied.
pub fn laid<R>(answer: Result<R, BackendError>) -> Result<Result<R, Occupied>, Error> {
    match answer {
        Ok(value) => Ok(Ok(value)),
        Err(BackendError::Exists(detail)) => Ok(Err(Occupied { detail })),
        Err(err) => Err(err.into()),
    }
}

fn sorted(mut paths: Vec<String>) -> Vec<String> {
    paths.sort();
    paths
}

// The entries a page holds: `limit`, capped at `MAX_PAGE_SIZE`, which is
// also what no `limit` asks for.
fn page_size(limit: Option<u32>) -> Result<usize, Error> {
    match limit {
        Some(0) => Err(Error::Invalid("a page `limit` must be at least 1".to_owned())),
        limit => Ok(limit.map_or(MAX_PAGE_SIZE, |limit| limit.min(MAX_PAGE_SIZE)) as usize),
    }
}

impl From<Change> for workspace::Change {
    fn from(change: Change) -> Self {
        Self {
            path: change.path,
            kind: change.kind.into(),
        }
    }
}

impl From<ChangeKind> for workspace::ChangeKind {
    fn from(kind: ChangeKind) -> Self {
        match kind {
            ChangeKind::Added => Self::Added,
            ChangeKind::Modified => Self::Modified,
            ChangeKind::Deleted => Self::Deleted,
        }
    }
}

impl From<Entry> for workspace::LogEntry {
    fn from(entry: Entry) -> Self {
        Self {
            id: entry.id,
            message: entry.message,
        }
    }
}

impl From<workspace::Rule> for Rule {
    fn from(rule: workspace::Rule) -> Self {
        Self {
            paths: rule.pattern,
            strategy: rule.strategy.into(),
        }
    }
}

impl From<workspace::Strategy> for Strategy {
    fn from(strategy: workspace::Strategy) -> Self {
        match strategy {
            workspace::Strategy::Union => Self::Union,
            workspace::Strategy::Ours => Self::Ours,
            workspace::Strategy::Theirs => Self::Theirs,
        }
    }
}

impl Host for HostView<'_, dyn VcsClient> {}

impl HostWorkspace for HostView<'_, dyn VcsClient> {}
