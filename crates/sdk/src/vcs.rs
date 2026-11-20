//! `omnia:vcs`, the repositories and working copies a guest lends the host.
//!
//! A guest names every workspace with a deployment-local path: `"."` the
//! project mount, `"./.cache/repo"` or `"/mount/sub"` beneath one, which need
//! not exist yet where the call creates it. [`open`], [`init`], [`clone`], and
//! [`Workspace::add`] lend the path to the host as an `omnia:vcs`
//! [`Location`](types::Location), resolved against the guest's preopens by
//! the lend rule [`Request::workspace`](crate::model::Request::workspace)
//! follows. A path no preopen answers is `denied` before the host is called.
//!
//! An open [`Workspace`] answers the calls of the `omnia:vcs` workspace
//! itself, such as [`Workspace::resolve`] and [`Workspace::merge`]. This
//! module adds the optional [`workspaces`] and [`remotes`] calls as its
//! methods, and [`Workspace::log_all`] and [`Workspace::status_all`], which
//! walk every page.

use std::fmt;

use wasip3::filesystem::preopens;
use wasip3::filesystem::types::Descriptor;

use crate::Result;
pub use crate::bindings::imports::omnia::vcs::remotes::Diverged;
pub use crate::bindings::imports::omnia::vcs::types::{CommitId, Occupied};
pub use crate::bindings::imports::omnia::vcs::workspace::{
    Change, ChangeKind, LogEntry, Rule, Strategy, Unmerged, Workspace,
};
pub use crate::bindings::imports::omnia::vcs::{remotes, store, types, workspace, workspaces};
use crate::lend::resolve_lend;

/// Open the workspace at `path`.
///
/// # Errors
///
/// Returns an [`Error`](crate::Error) when no workspace is at `path`, or the
/// host fails the call.
pub async fn open(path: &str) -> Result<Workspace> {
    let directories = preopens::get_directories();
    Ok(store::open(lend(&directories, path)?).await?)
}

/// Create a repository with no commits at `path` and open its workspace.
///
/// The files already at `path` are pending, and a missing directory is
/// created.
///
/// # Errors
///
/// Returns an [`Error`](crate::Error) when the host fails the call. An
/// [`Occupied`] is the inner `Err`: a workspace is already at `path`, and it
/// stays as it was.
pub async fn init(path: &str) -> Result<Result<Workspace, Occupied>> {
    let directories = preopens::get_directories();
    Ok(store::init(lend(&directories, path)?).await?)
}

/// Clone the repository at `url` into `path` and open its workspace.
///
/// The clone knows `url` as the remote `origin`. `depth` keeps that many
/// commits of each bookmark's history, and `None` keeps all of it.
///
/// # Errors
///
/// Returns an [`Error`](crate::Error) when `url` holds no repository, or the
/// host fails the call. An [`Occupied`] is the inner `Err`: the directory at
/// `path` holds anything, and it stays as it was.
pub async fn clone(
    url: &str, path: &str, depth: Option<u32>,
) -> Result<Result<Workspace, Occupied>> {
    let directories = preopens::get_directories();
    Ok(remotes::clone(url.to_owned(), lend(&directories, path)?, depth).await?)
}

impl Workspace {
    /// Add a workspace of the same repository at `path`, on the commit
    /// `revision` names, and open it.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`](crate::Error) when `revision` names no commit, or
    /// the host fails the call. An [`Occupied`] is the inner `Err`: the
    /// directory at `path` holds anything, and it stays as it was.
    pub async fn add(&self, path: &str, revision: &str) -> Result<Result<Self, Occupied>> {
        let directories = preopens::get_directories();
        Ok(workspaces::add(self, lend(&directories, path)?, revision.to_owned()).await?)
    }

    /// Remove this workspace and the files of its working copy, pending
    /// changes included.
    ///
    /// The handle then fails every call with `not-found`.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`](crate::Error) when this is the workspace [`init`]
    /// or [`clone`] made, or the host fails the call.
    pub async fn remove(&self) -> Result<()> {
        Ok(workspaces::remove(self).await?)
    }

    /// Bring the commits and bookmarks of `remote` into the repository.
    ///
    /// No bookmark of the repository moves.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`](crate::Error) when the repository does not know
    /// `remote`, the remote refuses or does not answer, or the host fails the
    /// call.
    pub async fn fetch(&self, remote: &str) -> Result<()> {
        Ok(remotes::fetch(self, remote.to_owned()).await?)
    }

    /// The commit the bookmark `name` of `remote` pointed at when the
    /// repository last saw it, `None` when it has seen no such bookmark.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`](crate::Error) when the repository does not know
    /// `remote`, or the host fails the call.
    pub async fn remote_bookmark(&self, remote: &str, name: &str) -> Result<Option<CommitId>> {
        Ok(remotes::bookmark(self, remote.to_owned(), name.to_owned()).await?)
    }

    /// Send the bookmark `name` and the commits it reaches to `remote`, and
    /// move the remote's bookmark of that name forward to the same commit.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`](crate::Error) when the repository does not know
    /// `remote` or `name`, the remote refuses or does not answer, or the host
    /// fails the call. A [`Diverged`] is the inner `Err`: the remote's
    /// bookmark holds commits the pushed one does not, and nothing changed.
    pub async fn push(&self, remote: &str, name: &str) -> Result<Result<(), Diverged>> {
        Ok(remotes::push(self, remote.to_owned(), name.to_owned()).await?)
    }

    /// Every entry of the log from `revision` back to `base`, newest first.
    ///
    /// [`Workspace::log`] sets out the log one page at a time.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`](crate::Error) when `revision` names no commit, or
    /// the host fails a page.
    pub async fn log_all(&self, revision: &str, base: Option<&str>) -> Result<Vec<LogEntry>> {
        let mut entries = Vec::new();
        let mut after = None;
        loop {
            let page =
                self.log(revision.to_owned(), base.map(ToOwned::to_owned), after, None).await?;
            entries.extend(page.entries);
            match page.next {
                Some(next) => after = Some(next),
                None => return Ok(entries),
            }
        }
    }

    /// Every pending change in the working copy, in path order.
    ///
    /// [`Workspace::status`] sets out the changes one page at a time.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`](crate::Error) when the host fails a page.
    pub async fn status_all(&self) -> Result<Vec<Change>> {
        let mut changes = Vec::new();
        let mut after = None;
        loop {
            let page = self.status(after, None).await?;
            changes.extend(page.changes);
            match page.next {
                Some(next) => after = Some(next),
                None => return Ok(changes),
            }
        }
    }
}

// the lent root borrows a descriptor, so the preopens outlive the call
fn lend<'a>(directories: &'a [(Descriptor, String)], path: &str) -> Result<types::Location<'a>> {
    let Some((root, subpath)) = resolve_lend(directories, path) else {
        return Err(types::Error::Denied(format!("location `{path}` matches no preopen")).into());
    };
    Ok(types::Location {
        root,
        subpath: subpath.to_owned(),
    })
}

impl fmt::Display for Occupied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the place is occupied: {}", self.detail)
    }
}

impl std::error::Error for Occupied {}

impl fmt::Display for Diverged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the remote's bookmark holds commits the push does not")?;
        self.current.as_ref().map_or(Ok(()), |current| write!(f, ", last seen at {current}"))
    }
}

impl std::error::Error for Diverged {}

impl fmt::Display for Unmerged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflicts(paths) => write!(f, "the merge left conflicts in {}", paths.join(", ")),
            Self::Pending(paths) => {
                write!(f, "the working copy holds pending changes in {}", paths.join(", "))
            }
        }
    }
}

impl std::error::Error for Unmerged {}
