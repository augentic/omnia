//! Service contract for `omnia:vcs` backends.

use std::fmt::Debug;

use super::{Change, CloneOptions, Entry, Merged, Place, Rule};
use crate::FutureResult;

/// Service contract for `omnia:vcs` backends.
pub trait VcsClient: Debug + Send + Sync + 'static {
    /// The commit `revision` names in the repository at `repo`.
    fn resolve(&self, repo: Place, revision: String) -> FutureResult<String>;

    /// Whether `ancestor` is in `descendant`'s history, itself included, in
    /// the repository at `repo`.
    fn descends(&self, repo: Place, ancestor: String, descendant: String) -> FutureResult<bool>;

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
