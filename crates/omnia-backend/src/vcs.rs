//! Version-control (`omnia:vcs`) service contract and value types.

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

#[cfg(not(target_arch = "wasm32"))]
mod place;

#[cfg(not(target_arch = "wasm32"))]
mod host {
    use std::fmt::Debug;

    use super::{Change, CloneOptions, Entry, Merged, Place, Rule};
    use crate::FutureResult;

    /// Service contract for `omnia:vcs` backends.
    pub trait VcsClient: Debug + Send + Sync + 'static {
        /// The commit `revision` names in the repository at `repo`.
        fn resolve(&self, repo: Place, revision: String) -> FutureResult<String>;

        /// Whether `ancestor` is in `descendant`'s history, itself included, in
        /// the repository at `repo`.
        fn descends(&self, repo: Place, ancestor: String, descendant: String)
        -> FutureResult<bool>;

        /// The sealed commit the working copy at `at` sits on.
        fn head(&self, at: Place) -> FutureResult<String>;

        /// Seal every pending change at `at` as one commit; `None` when there
        /// is nothing to seal.
        fn commit(&self, at: Place, message: String) -> FutureResult<Option<String>>;

        /// Merge `revision` into the working copy at `at` under `policy`.
        fn merge(
            &self, at: Place, revision: String, message: String, policy: Vec<Rule>,
        ) -> FutureResult<Merged>;

        /// The first-parent chain from `revision` back to `base` in the
        /// repository at `repo`, newest first, `base` left out.
        fn log(&self, repo: Place, revision: String, base: String) -> FutureResult<Vec<Entry>>;

        /// A repository with no history at `at`.
        fn init(&self, at: Place) -> FutureResult<()>;

        /// A working copy of `repo` at `at`, detached at `revision`.
        fn add(&self, repo: Place, at: Place, revision: String) -> FutureResult<()>;

        /// Remove the working copy at `at` and its files.
        fn remove(&self, at: Place) -> FutureResult<()>;

        /// What the working copy at `at` holds that its head does not.
        fn pending(&self, at: Place) -> FutureResult<Vec<Change>>;

        /// A clone of `url` at `at`.
        fn clone_repo(&self, url: String, at: Place, options: CloneOptions) -> FutureResult<()>;

        /// Bring `remote`'s commits and labels into the repository at `repo`.
        fn fetch(&self, repo: Place, remote: String) -> FutureResult<()>;

        /// Point `name` at `revision` in the repository at `repo`.
        fn label(&self, repo: Place, name: String, revision: String) -> FutureResult<()>;

        /// The commit the label `name` points at in the repository at `repo`,
        /// in the namespace `label` writes alone.
        fn labelled(&self, repo: Place, name: String) -> FutureResult<String>;

        /// The commit `remote`'s label `name` pointed at when the repository at
        /// `repo` last fetched from it, in the namespace `fetch` and `clone`
        /// write alone.
        fn fetched(&self, repo: Place, remote: String, name: String) -> FutureResult<String>;

        /// Send `label` and the commits it reaches to `remote`.
        fn push(&self, repo: Place, remote: String, label: String) -> FutureResult<()>;
    }

    impl VcsClient for Box<dyn VcsClient> {
        fn resolve(&self, repo: Place, revision: String) -> FutureResult<String> {
            (**self).resolve(repo, revision)
        }

        fn descends(
            &self, repo: Place, ancestor: String, descendant: String,
        ) -> FutureResult<bool> {
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
}

#[cfg(not(target_arch = "wasm32"))]
pub use self::host::VcsClient;
#[cfg(not(target_arch = "wasm32"))]
pub use self::place::{Intent, Place};

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn pending_lists_paths() {
        let error = Error::Pending(vec!["src/a.rs".into(), "b.md".into()]);
        assert_eq!(error.to_string(), "pending changes: src/a.rs, b.md");
    }
}
