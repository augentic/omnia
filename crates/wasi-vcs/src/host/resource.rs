//! The records a [`VcsClient`](crate::VcsClient) works in, and the [`Error`]
//! it fails with.

/// How a merge settles a path both sides changed, where a [`Rule`] matches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// Keep the lines of both sides, each once.
    Union,
    /// Keep the whole file as the head holds it.
    Ours,
    /// Keep the whole file as the merged-in commit holds it.
    Theirs,
}

/// A strategy for the paths a pattern matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// A pattern over paths relative to the working copy's root.
    pub paths: String,
    /// What settles a path the pattern matches.
    pub strategy: Strategy,
}

/// What a merge did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Merged {
    /// The commit the working copy is then on, `None` when a path is left in
    /// conflict.
    pub commit: Option<String>,
    /// The paths left in conflict when no commit was sealed.
    pub conflicts: Vec<String>,
}

/// One commit in a log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The commit's id.
    pub id: String,
    /// The message the commit holds.
    pub message: String,
}

/// How a path in a working copy differs from the head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// The head does not hold the path.
    Added,
    /// The head holds the path with other content.
    Modified,
    /// The head holds the path and the working copy does not.
    Deleted,
}

/// One pending change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// The path, relative to the working copy's root.
    pub path: String,
    /// How the path differs from the head.
    pub kind: ChangeKind,
}

/// Options for [`VcsClient::clone_repo`](crate::VcsClient::clone_repo).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CloneOptions {
    /// Keep this many commits of history, `None` for all of it.
    pub depth: Option<u32>,
}

/// Why a backend call failed.
///
/// A backend returns one inside its `anyhow` error.
/// [`VcsClient`](crate::VcsClient) lists the `omnia:vcs` case each variant
/// reaches the guest as.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The place holds no repository or working copy.
    #[error("not a repository")]
    NotARepository,
    /// A working copy already exists at the place.
    #[error("already exists: {0}")]
    Exists(String),
    /// A revision, remote, or bookmark the repository does not know.
    #[error("not found: {0}")]
    NotFound(String),
    /// A working copy holds pending changes where none may.
    #[error("pending changes: {}", .0.join(", "))]
    Pending(Vec<String>),
    /// The remote refused, could not be reached, or wants credentials.
    #[error("access: {0}")]
    Access(String),
    /// The remote's bookmark holds commits the pushed one does not.
    #[error("diverged: {0}")]
    Diverged(String),
    /// Any other failure, with the backend's detail.
    #[error("{0}")]
    Other(String),
}

impl Error {
    // The failure a backend returned, or `Other` with its full context chain
    // when it returned none.
    pub(crate) fn typed(err: anyhow::Error) -> Self {
        err.downcast().unwrap_or_else(|err| Self::Other(format!("{err:#}")))
    }
}
