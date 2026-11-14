//! [`Vcs`] over `omnia:vcs`.

use omnia_wasi_vcs::{store, transport, types, workspace};
use wasip3::filesystem::preopens;
use wasip3::filesystem::types::Descriptor;

use super::lend::resolve_lend;
use crate::vcs::{Change, ChangeKind, CloneOptions, Entry, Error, Merged, Rule, Strategy};
use crate::{Vcs, WasiProvider};

impl Vcs for WasiProvider {
    async fn resolve(&self, repo: &str, revision: &str) -> Result<String, Error> {
        let directories = preopens();
        let repo = lend(&directories, repo)?;
        Ok(store::resolve(repo, revision.to_owned()).await?)
    }

    async fn descends(&self, repo: &str, ancestor: &str, descendant: &str) -> Result<bool, Error> {
        let directories = preopens();
        let repo = lend(&directories, repo)?;
        Ok(store::descends(repo, ancestor.to_owned(), descendant.to_owned()).await?)
    }

    async fn head(&self, at: &str) -> Result<String, Error> {
        let directories = preopens();
        let at = lend(&directories, at)?;
        Ok(store::head(at).await?)
    }

    async fn commit(&self, at: &str, message: &str) -> Result<Option<String>, Error> {
        let directories = preopens();
        let at = lend(&directories, at)?;
        Ok(store::commit(at, message.to_owned()).await?)
    }

    async fn merge(
        &self, at: &str, revision: &str, message: &str, policy: &[Rule],
    ) -> Result<Merged, Error> {
        let policy: Vec<store::Rule> = policy.iter().cloned().map(Into::into).collect();
        let directories = preopens();
        let at = lend(&directories, at)?;
        let merged = store::merge(at, revision.to_owned(), message.to_owned(), policy).await?;
        Ok(merged.into())
    }

    async fn log(&self, repo: &str, revision: &str, base: &str) -> Result<Vec<Entry>, Error> {
        let directories = preopens();
        let repo = lend(&directories, repo)?;
        let entries = store::log(repo, revision.to_owned(), base.to_owned()).await?;
        Ok(entries.into_iter().map(Into::into).collect())
    }

    async fn init(&self, at: &str) -> Result<(), Error> {
        let directories = preopens();
        let at = lend(&directories, at)?;
        Ok(workspace::init(at).await?)
    }

    async fn add(&self, repo: &str, at: &str, revision: &str) -> Result<(), Error> {
        let directories = preopens();
        let repo = lend(&directories, repo)?;
        let at = lend(&directories, at)?;
        Ok(workspace::add(repo, at, revision.to_owned()).await?)
    }

    async fn remove(&self, at: &str) -> Result<(), Error> {
        let directories = preopens();
        let at = lend(&directories, at)?;
        Ok(workspace::remove(at).await?)
    }

    async fn pending(&self, at: &str) -> Result<Vec<Change>, Error> {
        let directories = preopens();
        let at = lend(&directories, at)?;
        let changes = workspace::pending(at).await?;
        Ok(changes.into_iter().map(Into::into).collect())
    }

    async fn clone_repo(&self, url: &str, at: &str, options: CloneOptions) -> Result<(), Error> {
        let directories = preopens();
        let at = lend(&directories, at)?;
        Ok(transport::clone(url.to_owned(), at, options.into()).await?)
    }

    async fn fetch(&self, repo: &str, remote: &str) -> Result<(), Error> {
        let directories = preopens();
        let repo = lend(&directories, repo)?;
        Ok(transport::fetch(repo, remote.to_owned()).await?)
    }

    async fn label(&self, repo: &str, name: &str, revision: &str) -> Result<(), Error> {
        let directories = preopens();
        let repo = lend(&directories, repo)?;
        Ok(transport::label(repo, name.to_owned(), revision.to_owned()).await?)
    }

    async fn labelled(&self, repo: &str, name: &str) -> Result<String, Error> {
        let directories = preopens();
        let repo = lend(&directories, repo)?;
        Ok(transport::labelled(repo, name.to_owned()).await?)
    }

    async fn fetched(&self, repo: &str, remote: &str, name: &str) -> Result<String, Error> {
        let directories = preopens();
        let repo = lend(&directories, repo)?;
        Ok(transport::fetched(repo, remote.to_owned(), name.to_owned()).await?)
    }

    async fn push(&self, repo: &str, remote: &str, label: &str) -> Result<(), Error> {
        let directories = preopens();
        let repo = lend(&directories, repo)?;
        Ok(transport::push(repo, remote.to_owned(), label.to_owned()).await?)
    }
}

// the lent root borrows a descriptor, so the preopens outlive the call
fn preopens() -> Vec<(Descriptor, String)> {
    preopens::get_directories()
}

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

impl From<Rule> for store::Rule {
    fn from(rule: Rule) -> Self {
        Self {
            paths: rule.paths,
            strategy: rule.strategy.into(),
        }
    }
}

impl From<Strategy> for store::Strategy {
    fn from(strategy: Strategy) -> Self {
        match strategy {
            Strategy::Union => Self::Union,
            Strategy::Ours => Self::Ours,
            Strategy::Theirs => Self::Theirs,
        }
    }
}

impl From<store::Merged> for Merged {
    fn from(merged: store::Merged) -> Self {
        Self {
            commit: merged.commit,
            conflicts: merged.conflicts,
        }
    }
}

impl From<store::Entry> for Entry {
    fn from(entry: store::Entry) -> Self {
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

impl From<CloneOptions> for transport::CloneOptions {
    fn from(options: CloneOptions) -> Self {
        Self { depth: options.depth }
    }
}

impl From<types::Error> for Error {
    fn from(error: types::Error) -> Self {
        match error {
            types::Error::NotARepository => Self::NotARepository,
            types::Error::Exists(detail) => Self::Exists(detail),
            types::Error::NotFound(detail) => Self::NotFound(detail),
            types::Error::Pending(paths) => Self::Pending(paths),
            types::Error::Access(detail) => Self::Access(detail),
            types::Error::Diverged(label) => Self::Diverged(label),
            types::Error::Other(detail) => Self::Other(detail),
        }
    }
}
