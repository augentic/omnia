#![doc = include_str!("../README.md")]

//! # WASI VCS
//!
//! This module implements the runtime boundary for `omnia:vcs`: a guest
//! opens a workspace at a location beneath its mounts, and the host carries
//! each call to a version-control backend, which works through an open
//! directory handle beneath the mount and never sees a descriptor.

mod location;
mod remotes_impl;
mod resource;
mod store_impl;
mod workspace_impl;
mod workspaces_impl;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

    pub use omnia_wasi_types::Error;

    pub use super::workspace_impl::WorkspaceHandle;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:clocks": wasmtime_wasi::p3::bindings::clocks,
            "wasi:filesystem": wasmtime_wasi::p3::bindings::filesystem,
            "omnia:vcs/workspace.workspace": WorkspaceHandle,
            "omnia:types/types": omnia_wasi_types::types,
        },
        trappable_error_type: {
            "omnia:types/types.error" => Error,
        },
    });
}

use std::fmt::Debug;

pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, HostView, Provides, StoreCtx};
use wasmtime::component::Linker;

use self::generated::omnia::vcs::types::Host as TypesHost;
use self::generated::omnia::vcs::{remotes, store, types, workspace, workspaces};
pub use self::location::Place;
pub use self::resource::*;

/// Result type for VCS operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:vcs`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = VcsClient)]
pub struct WasiVcs;

impl<B: Provides<Self>> Host<B> for WasiVcs {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        omnia_wasi_types::types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        workspace::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        store::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        workspaces::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(remotes::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}

/// Service contract for `omnia:vcs` backends.
///
/// This is implemented by the version-control backend. Each method works
/// through the [`Place`] a guest's `location` resolved to, so the backend
/// never sees a descriptor and the guest never sees a path. A place for
/// `init`, `clone`, or the working copy `add` lays down exists when the
/// backend runs, empty where nothing stood before.
///
/// The host serves the `omnia:vcs` interfaces over these methods:
///
/// - `open` calls [`head`](Self::head) to check that a workspace is at the
///   place. [`Error::NotFound`] there is a workspace with no commits.
/// - `merge` calls [`pending`](Self::pending) first, and merges only into a
///   working copy with nothing pending.
/// - `push` calls [`fetched`](Self::fetched) after a push that diverged,
///   for the commit its outcome reports.
/// - A lookup reads [`Error::NotFound`] as finding nothing.
/// - What these methods cannot express fails with `unsupported`: a `log`
///   with no base, and a bookmark precondition.
///
/// A backend picks the case of a failure by returning an [`Error`] inside
/// its `anyhow` error. The guest sees:
///
/// - [`Error::NotARepository`] and [`Error::NotFound`] as `not-found`;
/// - [`Error::Exists`] from [`init`](Self::init), [`add`](Self::add), and
///   [`clone_repo`](Self::clone_repo) as their `occupied` outcome;
/// - [`Error::Pending`] from [`merge`](Self::merge) as its `pending`
///   outcome;
/// - [`Error::Diverged`] from [`push`](Self::push) as its `diverged`
///   outcome;
/// - [`Error::Denied`] as `denied`;
/// - [`Error::Access`] as `unavailable`;
/// - any other failure as `other`, with its full context chain.
///
/// The `remotes` operation `clone` is `clone_repo` here, since `clone(&self,
/// ..)` would shadow [`Clone::clone`] on every backend that is `Clone`.
pub trait VcsClient: Debug + Send + Sync + 'static {
    /// The commit `revision` names in the repository at `repo`.
    fn resolve(&self, repo: Place, revision: String) -> FutureResult<String>;

    /// Whether `ancestor` is in `descendant`'s history, itself included, in
    /// the repository at `repo`.
    fn descends(&self, repo: Place, ancestor: String, descendant: String) -> FutureResult<bool>;

    /// The commit the working copy at `at` is on.
    ///
    /// A working copy with no commit yet is [`Error::NotFound`], and a place
    /// that holds no working copy is [`Error::NotARepository`].
    fn head(&self, at: Place) -> FutureResult<String>;

    /// Seal every pending change at `at` as one commit, `None` when nothing
    /// is pending.
    fn commit(&self, at: Place, message: String) -> FutureResult<Option<String>>;

    /// Merge `revision` into the working copy at `at` under `policy`.
    fn merge(
        &self, at: Place, revision: String, message: String, policy: Vec<Rule>,
    ) -> FutureResult<Merged>;

    /// The first-parent chain from `revision` back to `base` in the
    /// repository at `repo`, newest first, `base` left out.
    fn log(&self, repo: Place, revision: String, base: String) -> FutureResult<Vec<Entry>>;

    /// A repository with no commits at `at`.
    ///
    /// A place that already holds one is [`Error::Exists`].
    fn init(&self, at: Place) -> FutureResult<()>;

    /// A working copy of `repo` at `at`, on the commit `revision` names.
    ///
    /// A place that holds anything is [`Error::Exists`].
    fn add(&self, repo: Place, at: Place, revision: String) -> FutureResult<()>;

    /// Remove the working copy at `at` and its files.
    fn remove(&self, at: Place) -> FutureResult<()>;

    /// What the working copy at `at` holds that its head does not.
    fn pending(&self, at: Place) -> FutureResult<Vec<Change>>;

    /// A clone of `url` at `at`.
    ///
    /// A place that holds anything is [`Error::Exists`].
    fn clone_repo(&self, url: String, at: Place, options: CloneOptions) -> FutureResult<()>;

    /// Bring `remote`'s commits and bookmarks into the repository at `repo`.
    fn fetch(&self, repo: Place, remote: String) -> FutureResult<()>;

    /// Point the bookmark `name` at `revision` in the repository at `repo`.
    fn label(&self, repo: Place, name: String, revision: String) -> FutureResult<()>;

    /// The commit the bookmark `name` points at in the repository at `repo`,
    /// in the namespace [`label`](Self::label) writes alone.
    fn labelled(&self, repo: Place, name: String) -> FutureResult<String>;

    /// The commit `remote`'s bookmark `name` pointed at when the repository
    /// at `repo` last saw it.
    ///
    /// A remote the repository does not know is [`Error::NotFound`] naming
    /// the remote, and a bookmark it has not seen is one naming the bookmark.
    fn fetched(&self, repo: Place, remote: String, name: String) -> FutureResult<String>;

    /// Send the bookmark `label` and the commits it reaches to `remote`.
    ///
    /// A remote bookmark the push would move backwards is
    /// [`Error::Diverged`].
    fn push(&self, repo: Place, remote: String, label: String) -> FutureResult<()>;
}

impl VcsClient for Box<dyn VcsClient> {
    fn resolve(&self, repo: Place, revision: String) -> FutureResult<String> {
        (**self).resolve(repo, revision)
    }

    fn descends(&self, repo: Place, ancestor: String, descendant: String) -> FutureResult<bool> {
        (**self).descends(repo, ancestor, descendant)
    }

    fn head(&self, at: Place) -> FutureResult<String> {
        (**self).head(at)
    }

    fn commit(&self, at: Place, message: String) -> FutureResult<Option<String>> {
        (**self).commit(at, message)
    }

    fn merge(
        &self, at: Place, revision: String, message: String, policy: Vec<Rule>,
    ) -> FutureResult<Merged> {
        (**self).merge(at, revision, message, policy)
    }

    fn log(&self, repo: Place, revision: String, base: String) -> FutureResult<Vec<Entry>> {
        (**self).log(repo, revision, base)
    }

    fn init(&self, at: Place) -> FutureResult<()> {
        (**self).init(at)
    }

    fn add(&self, repo: Place, at: Place, revision: String) -> FutureResult<()> {
        (**self).add(repo, at, revision)
    }

    fn remove(&self, at: Place) -> FutureResult<()> {
        (**self).remove(at)
    }

    fn pending(&self, at: Place) -> FutureResult<Vec<Change>> {
        (**self).pending(at)
    }

    fn clone_repo(&self, url: String, at: Place, options: CloneOptions) -> FutureResult<()> {
        (**self).clone_repo(url, at, options)
    }

    fn fetch(&self, repo: Place, remote: String) -> FutureResult<()> {
        (**self).fetch(repo, remote)
    }

    fn label(&self, repo: Place, name: String, revision: String) -> FutureResult<()> {
        (**self).label(repo, name, revision)
    }

    fn labelled(&self, repo: Place, name: String) -> FutureResult<String> {
        (**self).labelled(repo, name)
    }

    fn fetched(&self, repo: Place, remote: String, name: String) -> FutureResult<String> {
        (**self).fetched(repo, remote, name)
    }

    fn push(&self, repo: Place, remote: String, label: String) -> FutureResult<()> {
        (**self).push(repo, remote, label)
    }
}

impl From<Error> for generated::Error {
    fn from(err: Error) -> Self {
        match err {
            Error::NotARepository => Self::NotFound(err.to_string()),
            Error::NotFound(name) => Self::NotFound(name),
            Error::Denied(detail) => Self::Denied(detail),
            Error::Access(detail) => Self::Unavailable(detail),
            Error::Exists(_) | Error::Pending(_) | Error::Diverged(_) => {
                Self::Other(err.to_string())
            }
            Error::Other(detail) => Self::Other(detail),
        }
    }
}

impl TypesHost for HostView<'_, dyn VcsClient> {}
