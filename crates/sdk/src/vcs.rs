//! `omnia:vcs`, the repositories and working copies a guest lends the host.
//!
//! A guest names every repository and working copy with a deployment-local
//! path: `"."` the project mount, `"./.cache/repo"` or `"/mount/sub"`
//! beneath one, which need not exist yet where the operation creates it.
//! Each fn here lends the path to the host as an `omnia:vcs` `location`,
//! resolved against the guest's preopens by the lend rule
//! [`Request::workspace`](crate::model::Request::workspace) follows.
//!
//! Every fn but [`init`] and [`clone`] opens the workspace at its path, and a
//! path that holds none is [`Error::NotARepository`]. A fn that walks pages,
//! [`log`] and [`pending`], returns them whole. The raw interfaces are
//! [`store`], [`workspace`], [`workspaces`], and [`remotes`].

use wasip3::filesystem::preopens;
use wasip3::filesystem::types::Descriptor;

pub use crate::bindings::imports::omnia::vcs::{remotes, store, types, workspace, workspaces};
use crate::lend::resolve_lend;
use crate::vcs::workspace::{Unmerged, Workspace};

/// The commit `revision` names in the repository at `repo`: a label, a tag,
/// or a commit id, whole or a unique prefix.
///
/// # Errors
///
/// [`Error::NotFound`] for a revision the repository does not know, or the
/// [`Error`] the host reports.
pub async fn resolve(repo: &str, revision: &str) -> Result<String, Error> {
    let found = open(repo).await?.resolve(revision.to_owned()).await?;
    found.ok_or_else(|| Error::NotFound(revision.to_owned()))
}

/// Whether `ancestor` is in `descendant`'s history, itself included, in the
/// repository at `repo`.
///
/// # Errors
///
/// The [`Error`] the host reports.
pub async fn descends(repo: &str, ancestor: &str, descendant: &str) -> Result<bool, Error> {
    let workspace = open(repo).await?;
    Ok(workspace.is_ancestor(ancestor.to_owned(), descendant.to_owned()).await?)
}

/// The sealed commit the working copy at `at` sits on.
///
/// # Errors
///
/// [`Error::NotFound`] for a working copy with no commit yet, or the
/// [`Error`] the host reports.
pub async fn head(at: &str) -> Result<String, Error> {
    let head = open(at).await?.head().await?;
    head.ok_or_else(|| Error::NotFound("HEAD".to_owned()))
}

/// Seal every pending change at `at` as one commit, `None` when there is
/// nothing to seal.
///
/// # Errors
///
/// The [`Error`] the host reports.
pub async fn commit(at: &str, message: &str) -> Result<Option<String>, Error> {
    Ok(open(at).await?.commit(message.to_owned()).await?)
}

/// Merge `revision` into the working copy at `at` under `policy`, and
/// advance its head.
///
/// A conflict no rule resolves is data: the [`Merged`] names its paths, and
/// the working copy is back on its head.
///
/// # Errors
///
/// [`Error::Pending`] naming the paths of a working copy with pending
/// changes, which changes nothing, or the [`Error`] the host reports.
pub async fn merge(
    at: &str, revision: &str, message: &str, policy: &[Rule],
) -> Result<Merged, Error> {
    let policy = policy.iter().cloned().map(Into::into).collect();
    let workspace = open(at).await?;
    let merged = workspace.merge(revision.to_owned(), message.to_owned(), policy).await?;
    match merged {
        Ok(commit) => Ok(Merged {
            commit: Some(commit),
            conflicts: Vec::new(),
        }),
        Err(Unmerged::Conflicts(paths)) => Ok(Merged {
            commit: None,
            conflicts: paths,
        }),
        Err(Unmerged::Pending(paths)) => Err(Error::Pending(paths)),
    }
}

/// The first-parent chain from `revision` back to `base` in the repository
/// at `repo`, newest first, `base` left out.
///
/// # Errors
///
/// The [`Error`] the host reports.
pub async fn log(repo: &str, revision: &str, base: &str) -> Result<Vec<Entry>, Error> {
    let workspace = open(repo).await?;
    let mut entries = Vec::new();
    let mut after = None;
    loop {
        let page = workspace.log(revision.to_owned(), Some(base.to_owned()), after, None).await?;
        entries.extend(page.entries.into_iter().map(Into::into));
        match page.next {
            Some(next) => after = Some(next),
            None => return Ok(entries),
        }
    }
}

/// Create a repository with no history at `at`.
///
/// # Errors
///
/// [`Error::Exists`] when a repository is already at `at`, which stays as
/// it was, or the [`Error`] the host reports.
pub async fn init(at: &str) -> Result<(), Error> {
    let directories = preopens::get_directories();
    store::init(lend(&directories, at)?).await??;
    Ok(())
}

/// Lay a working copy of `repo` at `at`, on `revision` and on no label.
///
/// # Errors
///
/// [`Error::Exists`] when a directory at `at` holds anything, which stays as
/// it was, or the [`Error`] the host reports.
pub async fn add(repo: &str, at: &str, revision: &str) -> Result<(), Error> {
    let workspace = open(repo).await?;
    let directories = preopens::get_directories();
    workspaces::add(&workspace, lend(&directories, at)?, revision.to_owned()).await??;
    Ok(())
}

/// Remove the working copy at `at` and its files, pending changes included.
///
/// # Errors
///
/// The [`Error`] the host reports.
pub async fn remove(at: &str) -> Result<(), Error> {
    let workspace = open(at).await?;
    Ok(workspaces::remove(&workspace).await?)
}

/// What the working copy at `at` holds that its head does not.
///
/// # Errors
///
/// The [`Error`] the host reports.
pub async fn pending(at: &str) -> Result<Vec<Change>, Error> {
    let workspace = open(at).await?;
    let mut changes = Vec::new();
    let mut after = None;
    loop {
        let page = workspace.status(after, None).await?;
        changes.extend(page.changes.into_iter().map(Into::into));
        match page.next {
            Some(next) => after = Some(next),
            None => return Ok(changes),
        }
    }
}

/// Clone `url` at `at`.
///
/// # Errors
///
/// [`Error::Exists`] when a directory at `at` holds anything, which stays as
/// it was, or the [`Error`] the host reports.
pub async fn clone(url: &str, at: &str, options: CloneOptions) -> Result<(), Error> {
    let directories = preopens::get_directories();
    remotes::clone(url.to_owned(), lend(&directories, at)?, options.depth).await??;
    Ok(())
}

/// Bring `remote`'s commits and labels into the repository at `repo`.
///
/// # Errors
///
/// [`Error::Access`] when the remote refuses or cannot be reached, or the
/// [`Error`] the host reports.
pub async fn fetch(repo: &str, remote: &str) -> Result<(), Error> {
    let workspace = open(repo).await?;
    Ok(remotes::fetch(&workspace, remote.to_owned()).await?)
}

/// Point the label `name` at `revision` in the repository at `repo`,
/// creating or moving it.
///
/// # Errors
///
/// The [`Error`] the host reports.
pub async fn label(repo: &str, name: &str, revision: &str) -> Result<(), Error> {
    let workspace = open(repo).await?;
    workspace.set_bookmark(name.to_owned(), revision.to_owned(), None).await??;
    Ok(())
}

/// The commit the label `name` points at in the repository at `repo`, in the
/// namespace [`label`] writes alone.
///
/// # Errors
///
/// [`Error::NotFound`] for a label the repository does not hold, or the
/// [`Error`] the host reports.
pub async fn labelled(repo: &str, name: &str) -> Result<String, Error> {
    let found = open(repo).await?.bookmark(name.to_owned()).await?;
    found.ok_or_else(|| Error::NotFound(name.to_owned()))
}

/// The commit `remote`'s label `name` pointed at when the repository at
/// `repo` last fetched from it, in the namespace [`fetch`] and [`clone`]
/// write alone.
///
/// # Errors
///
/// [`Error::NotFound`] for a label the remote did not hold, or the
/// [`Error`] the host reports.
pub async fn fetched(repo: &str, remote: &str, name: &str) -> Result<String, Error> {
    let workspace = open(repo).await?;
    let found = remotes::bookmark(&workspace, remote.to_owned(), name.to_owned()).await?;
    found.ok_or_else(|| Error::NotFound(name.to_owned()))
}

/// Send `label` and the commits it reaches to `remote`, forcing nothing.
///
/// # Errors
///
/// [`Error::Diverged`] when the remote's label holds commits the pushed one
/// does not, or the [`Error`] the host reports.
pub async fn push(repo: &str, remote: &str, label: &str) -> Result<(), Error> {
    let workspace = open(repo).await?;
    let pushed = remotes::push(&workspace, remote.to_owned(), label.to_owned()).await?;
    pushed.map_err(|_diverged| Error::Diverged(label.to_owned()))
}

// a path that holds no workspace is not a repository
async fn open(path: &str) -> Result<Workspace, Error> {
    let directories = preopens::get_directories();
    store::open(lend(&directories, path)?).await.map_err(|err| match err {
        types::Error::NotFound(_) => Error::NotARepository,
        err => err.into(),
    })
}

// the lent root borrows a descriptor, so the preopens outlive the call
fn lend<'a>(
    directories: &'a [(Descriptor, String)], path: &str,
) -> Result<types::Location<'a>, Error> {
    let (root, subpath) = resolve_lend(directories, path)
        .ok_or_else(|| Error::Other(format!("location `{path}` matches no preopen")))?;
    Ok(types::Location {
        root,
        subpath: subpath.to_owned(),
    })
}

/// How a merge resolves a conflict at a path a [`Rule`] matches, before the
/// conflict is reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// Both sides' lines kept, each once: declaration and import lists.
    Union,
    /// The working copy's side kept whole, for the caller to regenerate:
    /// lockfiles.
    Ours,
    /// The merged-in side kept whole.
    Theirs,
}

/// One merge rule: a glob over root-relative paths and the strategy for
/// the paths it matches. The first rule that matches applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The glob, over paths relative to the working copy's root.
    pub paths: String,
    /// How a conflict at a matching path is resolved.
    pub strategy: Strategy,
}

/// The outcome of a merge: data, not an error.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Merged {
    /// The merge commit, when the merge completed.
    pub commit: Option<String>,
    /// The paths in conflict no rule resolved, when it did not; the
    /// working copy is back on its head.
    pub conflicts: Vec<String>,
}

/// One entry of a [`log`]: a sealed commit's id and its whole message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The commit's id.
    pub id: String,
    /// The whole message, the trailers a caller wrote into it included.
    pub message: String,
}

/// What a working copy did to a path its head does not hold that way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// The path is new.
    Added,
    /// The path's content differs.
    Modified,
    /// The path is gone.
    Deleted,
}

/// One pending change in a working copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// The path, relative to the working copy's root.
    pub path: String,
    /// What the working copy did to it.
    pub kind: ChangeKind,
}

/// Options for a clone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CloneOptions {
    /// Clone only this many commits of history; `None` clones it whole.
    pub depth: Option<u32>,
}

/// Why a version-control call failed.
///
/// A refusal that has no variant here, such as a change beneath a read-only
/// mount, is [`Error::Other`] led by its `omnia:vcs` case.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The location holds no repository or working copy.
    #[error("not a repository")]
    NotARepository,
    /// The location already holds a repository, or files a new working copy
    /// would overwrite.
    #[error("already exists: {0}")]
    Exists(String),
    /// A revision, remote, or label the repository does not know.
    #[error("not found: {0}")]
    NotFound(String),
    /// A working copy holds pending changes where none may.
    #[error("pending changes: {}", .0.join(", "))]
    Pending(Vec<String>),
    /// The remote refused, could not be reached, or wants credentials.
    #[error("access: {0}")]
    Access(String),
    /// The remote's label holds commits the pushed one does not; nothing
    /// was forced.
    #[error("diverged: {0}")]
    Diverged(String),
    /// Any other failure, with the backend's detail.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// The kebab-case wire discriminant, stable for callers to branch on.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotARepository => "not-a-repository",
            Self::Exists(_) => "exists",
            Self::NotFound(_) => "not-found",
            Self::Pending(_) => "pending",
            Self::Access(_) => "access",
            Self::Diverged(_) => "diverged",
            Self::Other(_) => "other",
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        error.downcast().unwrap_or_else(|other| Self::Other(format!("{other:#}")))
    }
}

// The default taxonomy: what the repository lacks is `not_found`, a remote
// that cannot be reached is the gateway's, a location, working copy, or
// label in the wrong state is the request's, and the rest is the host's.
impl From<Error> for crate::Error {
    fn from(error: Error) -> Self {
        let code = error.code().to_owned();
        let description = error.to_string();
        match error {
            Error::NotFound(_) => Self::NotFound { code, description },
            Error::Access(_) => Self::BadGateway { code, description },
            Error::Other(_) => Self::ServerError { code, description },
            Error::NotARepository | Error::Exists(_) | Error::Pending(_) | Error::Diverged(_) => {
                Self::BadRequest { code, description }
            }
        }
    }
}

impl From<Rule> for workspace::Rule {
    fn from(rule: Rule) -> Self {
        Self {
            pattern: rule.paths,
            strategy: rule.strategy.into(),
        }
    }
}

impl From<Strategy> for workspace::Strategy {
    fn from(strategy: Strategy) -> Self {
        match strategy {
            Strategy::Union => Self::Union,
            Strategy::Ours => Self::Ours,
            Strategy::Theirs => Self::Theirs,
        }
    }
}

impl From<workspace::LogEntry> for Entry {
    fn from(entry: workspace::LogEntry) -> Self {
        Self {
            id: entry.id,
            message: entry.message,
        }
    }
}

impl From<workspace::Change> for Change {
    fn from(change: workspace::Change) -> Self {
        Self {
            path: change.path,
            kind: change.kind.into(),
        }
    }
}

impl From<workspace::ChangeKind> for ChangeKind {
    fn from(kind: workspace::ChangeKind) -> Self {
        match kind {
            workspace::ChangeKind::Added => Self::Added,
            workspace::ChangeKind::Modified => Self::Modified,
            workspace::ChangeKind::Deleted => Self::Deleted,
        }
    }
}

// `denied`, `invalid`, and `unsupported` have no variant of their own, so
// their case leads the detail
impl From<types::Error> for Error {
    fn from(err: types::Error) -> Self {
        match err {
            types::Error::NotFound(detail) => Self::NotFound(detail),
            types::Error::Denied(detail) => Self::Other(format!("denied: {detail}")),
            types::Error::Invalid(detail) => Self::Other(format!("invalid: {detail}")),
            types::Error::Unsupported(detail) => Self::Other(format!("unsupported: {detail}")),
            types::Error::Unavailable(detail) => Self::Access(detail),
            types::Error::Other(detail) => Self::Other(detail),
        }
    }
}

// a move with no precondition has none to fail
impl From<types::Conflict> for Error {
    fn from(_: types::Conflict) -> Self {
        Self::Other("the host reported a conflict on an unconditional bookmark move".to_owned())
    }
}

impl From<types::Occupied> for Error {
    fn from(occupied: types::Occupied) -> Self {
        Self::Exists(occupied.detail)
    }
}
