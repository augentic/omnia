//! [`Vcs`] over `omnia:vcs`.

use omnia_wasi_vcs::omnia::workspace::{self, Workspace};
use omnia_wasi_vcs::omnia::{remotes, store, types, workspaces};
use wasip3::filesystem::preopens;
use wasip3::filesystem::types::Descriptor;

use super::lend::resolve_lend;
use crate::vcs::{Change, ChangeKind, CloneOptions, Entry, Error, Merged, Rule, Strategy};
use crate::{Vcs, WasiProvider};

impl Vcs for WasiProvider {
    async fn resolve(&self, repo: &str, revision: &str) -> Result<String, Error> {
        let found = open(repo).await?.resolve(revision.to_owned()).await?;
        found.ok_or_else(|| Error::NotFound(revision.to_owned()))
    }

    async fn descends(&self, repo: &str, ancestor: &str, descendant: &str) -> Result<bool, Error> {
        let workspace = open(repo).await?;
        Ok(workspace.is_ancestor(ancestor.to_owned(), descendant.to_owned()).await?)
    }

    async fn head(&self, at: &str) -> Result<String, Error> {
        let head = open(at).await?.head().await?;
        head.ok_or_else(|| Error::NotFound("HEAD".to_owned()))
    }

    async fn commit(&self, at: &str, message: &str) -> Result<Option<String>, Error> {
        Ok(open(at).await?.commit(message.to_owned()).await?)
    }

    async fn merge(
        &self, at: &str, revision: &str, message: &str, policy: &[Rule],
    ) -> Result<Merged, Error> {
        let policy = policy.iter().cloned().map(Into::into).collect();
        let workspace = open(at).await?;
        let merged = workspace.merge(revision.to_owned(), message.to_owned(), policy).await?;
        Ok(match merged {
            Ok(commit) => Merged {
                commit: Some(commit),
                conflicts: Vec::new(),
            },
            Err(conflict) => Merged {
                commit: None,
                conflicts: conflict.paths,
            },
        })
    }

    async fn log(&self, repo: &str, revision: &str, base: &str) -> Result<Vec<Entry>, Error> {
        let workspace = open(repo).await?;
        let mut entries = Vec::new();
        let mut after = None;
        loop {
            let page =
                workspace.log(revision.to_owned(), Some(base.to_owned()), after, None).await?;
            entries.extend(page.entries.into_iter().map(Into::into));
            match page.next {
                Some(next) => after = Some(next),
                None => return Ok(entries),
            }
        }
    }

    async fn init(&self, at: &str) -> Result<(), Error> {
        let directories = preopens();
        store::init(lend(&directories, at)?).await?;
        Ok(())
    }

    async fn add(&self, repo: &str, at: &str, revision: &str) -> Result<(), Error> {
        let workspace = open(repo).await?;
        let directories = preopens();
        workspaces::add(&workspace, lend(&directories, at)?, revision.to_owned()).await?;
        Ok(())
    }

    async fn remove(&self, at: &str) -> Result<(), Error> {
        let workspace = open(at).await?;
        Ok(workspaces::remove(&workspace).await?)
    }

    async fn pending(&self, at: &str) -> Result<Vec<Change>, Error> {
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

    async fn clone_repo(&self, url: &str, at: &str, options: CloneOptions) -> Result<(), Error> {
        let directories = preopens();
        remotes::clone(url.to_owned(), lend(&directories, at)?, options.depth).await?;
        Ok(())
    }

    async fn fetch(&self, repo: &str, remote: &str) -> Result<(), Error> {
        let workspace = open(repo).await?;
        Ok(remotes::fetch(&workspace, remote.to_owned()).await?)
    }

    async fn label(&self, repo: &str, name: &str, revision: &str) -> Result<(), Error> {
        let workspace = open(repo).await?;
        workspace.set_bookmark(name.to_owned(), revision.to_owned(), None).await??;
        Ok(())
    }

    async fn labelled(&self, repo: &str, name: &str) -> Result<String, Error> {
        let found = open(repo).await?.bookmark(name.to_owned()).await?;
        found.ok_or_else(|| Error::NotFound(name.to_owned()))
    }

    async fn fetched(&self, repo: &str, remote: &str, name: &str) -> Result<String, Error> {
        let workspace = open(repo).await?;
        let found = remotes::bookmark(&workspace, remote.to_owned(), name.to_owned()).await?;
        found.ok_or_else(|| Error::NotFound(name.to_owned()))
    }

    async fn push(&self, repo: &str, remote: &str, label: &str) -> Result<(), Error> {
        let workspace = open(repo).await?;
        let pushed = remotes::push(&workspace, remote.to_owned(), label.to_owned()).await?;
        pushed.map_err(|diverged| Error::Diverged(diverged.bookmark))
    }
}

// A path that holds no workspace is not a repository.
async fn open(path: &str) -> Result<Workspace, Error> {
    let directories = preopens();
    store::open(lend(&directories, path)?).await.map_err(|err| match err {
        types::Error::NotFound(_) => Error::NotARepository,
        err => err.into(),
    })
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
