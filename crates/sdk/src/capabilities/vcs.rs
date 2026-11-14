//! Version-control capability over `omnia:vcs`.
//!
//! Target-independent mirrors of the `omnia:vcs` records, and the [`Vcs`]
//! trait over them. The one record that cannot cross off `wasm32` is the
//! `location` — a borrowed `wasi:filesystem` descriptor plus a subpath — so
//! a guest names every repository and working copy with a deployment-local
//! path (`"."` the project mount, `"./.cache/repo"` or `"/mount/sub"`
//! beneath one) and [`WasiProvider`](crate::WasiProvider) resolves it against
//! the guest's preopens at the call site, by the lend rule [`Model`] uses for
//! its workspace.
//!
//! [`Model`]: crate::Model

use std::future::Future;

/// Version control (Omnia VCS).
///
/// Every location is a deployment-local path resolved against the guest's
/// preopens on `wasm32`: `"."` names the project mount, `"./sub"` or
/// `"/mount/sub"` a directory beneath one, which need not exist yet where
/// the operation creates it. [`WasiProvider`](crate::WasiProvider) delegates
/// to `omnia:vcs` via `omnia-wasi-vcs`.
///
/// The transport operation `clone` is [`clone_repo`](Self::clone_repo)
/// here, since `clone(&self, ..)` would shadow [`Clone::clone`] on every
/// provider that is `Clone`.
#[omnia_guest_macros::capability]
pub trait Vcs: Send + Sync {
    /// The commit `revision` names in the repository at `repo`: a
    /// label, a tag, or a commit id, whole or a unique prefix.
    fn resolve(
        &self, repo: &str, revision: &str,
    ) -> impl Future<Output = Result<String, Error>> + Send;

    /// Whether `ancestor` is in `descendant`'s history, itself included,
    /// in the repository at `repo`.
    fn descends(
        &self, repo: &str, ancestor: &str, descendant: &str,
    ) -> impl Future<Output = Result<bool, Error>> + Send;

    /// The sealed commit the working copy at `at` sits on; one with
    /// no commit yet is [`Error::NotFound`].
    fn head(&self, at: &str) -> impl Future<Output = Result<String, Error>> + Send;

    /// Seals every pending change at `at` as one commit; `None` when
    /// there is nothing to seal.
    fn commit(
        &self, at: &str, message: &str,
    ) -> impl Future<Output = Result<Option<String>, Error>> + Send;

    /// Merges `revision` into the working copy at `at` under `policy`
    /// and advances its head.
    fn merge(
        &self, at: &str, revision: &str, message: &str, policy: &[Rule],
    ) -> impl Future<Output = Result<Merged, Error>> + Send;

    /// The first-parent chain from `revision` back to `base` in the
    /// repository at `repo`, newest first, `base` left out.
    fn log(
        &self, repo: &str, revision: &str, base: &str,
    ) -> impl Future<Output = Result<Vec<Entry>, Error>> + Send;

    /// A repository with no history at `at`.
    fn init(&self, at: &str) -> impl Future<Output = Result<(), Error>> + Send;

    /// A working copy of `repo` at `at`, on `revision` and on no label.
    fn add(
        &self, repo: &str, at: &str, revision: &str,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Removes the working copy at `at` and its files; pending changes
    /// go with it.
    fn remove(&self, at: &str) -> impl Future<Output = Result<(), Error>> + Send;

    /// What the working copy at `at` holds that its head does not.
    fn pending(&self, at: &str) -> impl Future<Output = Result<Vec<Change>, Error>> + Send;

    /// A clone of `url` at `at`, which must hold nothing yet.
    fn clone_repo(
        &self, url: &str, at: &str, options: CloneOptions,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Brings `remote`'s commits and labels into the repository at `repo`.
    fn fetch(&self, repo: &str, remote: &str) -> impl Future<Output = Result<(), Error>> + Send;

    /// Points `name` at `revision` in the repository at `repo`, creating
    /// or moving it.
    fn label(
        &self, repo: &str, name: &str, revision: &str,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// The commit the label `name` points at in the repository at
    /// `repo`, in the namespace [`label`](Self::label) writes alone.
    fn labelled(
        &self, repo: &str, name: &str,
    ) -> impl Future<Output = Result<String, Error>> + Send;

    /// The commit `remote`'s label `name` pointed at when the
    /// repository at `repo` last fetched from it, in the namespace
    /// [`fetch`](Self::fetch) and [`clone_repo`](Self::clone_repo)
    /// write alone.
    fn fetched(
        &self, repo: &str, remote: &str, name: &str,
    ) -> impl Future<Output = Result<String, Error>> + Send;

    /// Sends `label` and the commits it reaches to `remote`; nothing is
    /// forced.
    fn push(
        &self, repo: &str, remote: &str, label: &str,
    ) -> impl Future<Output = Result<(), Error>> + Send;
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

/// One entry of a [`Vcs::log`]: a sealed commit's id and its whole message.
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

/// Typed version-control failure, mirroring the `omnia:vcs` error variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The location holds no repository or working copy.
    #[error("not a repository")]
    NotARepository,
    /// A working copy already exists at the location.
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
