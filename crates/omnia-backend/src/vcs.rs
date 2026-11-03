//! Version-control (`omnia:vcs`) service contract and value types.

#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(not(target_arch = "wasm32"))]
mod place;

#[cfg(not(target_arch = "wasm32"))]
pub use self::client::VcsClient;
#[cfg(not(target_arch = "wasm32"))]
pub use self::place::{Intent, Place};

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

/// One entry of a commit log: a sealed commit's id and its whole message.
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

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn pending_lists_paths() {
        let error = Error::Pending(vec!["src/a.rs".into(), "b.md".into()]);
        assert_eq!(error.to_string(), "pending changes: src/a.rs, b.md");
    }
}
