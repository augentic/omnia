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

pub use omnia_backend::vcs::{
    Change, ChangeKind, CloneOptions, Entry, Error, Merged, Rule, Strategy,
};

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

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Vcs for crate::WasiProvider {
    async fn resolve(&self, repo: &str, revision: &str) -> Result<String, Error> {
        let directories = wire::preopens();
        let repo = wire::lend(&directories, repo)?;
        Ok(omnia_wasi_vcs::store::resolve(repo, revision.to_owned()).await?)
    }

    async fn descends(&self, repo: &str, ancestor: &str, descendant: &str) -> Result<bool, Error> {
        let directories = wire::preopens();
        let repo = wire::lend(&directories, repo)?;
        Ok(omnia_wasi_vcs::store::descends(repo, ancestor.to_owned(), descendant.to_owned())
            .await?)
    }

    async fn head(&self, at: &str) -> Result<String, Error> {
        let directories = wire::preopens();
        let at = wire::lend(&directories, at)?;
        Ok(omnia_wasi_vcs::store::head(at).await?)
    }

    async fn commit(&self, at: &str, message: &str) -> Result<Option<String>, Error> {
        let directories = wire::preopens();
        let at = wire::lend(&directories, at)?;
        Ok(omnia_wasi_vcs::store::commit(at, message.to_owned()).await?)
    }

    async fn merge(
        &self, at: &str, revision: &str, message: &str, policy: &[Rule],
    ) -> Result<Merged, Error> {
        let policy: Vec<omnia_wasi_vcs::store::Rule> =
            policy.iter().cloned().map(Into::into).collect();
        let directories = wire::preopens();
        let at = wire::lend(&directories, at)?;
        let merged =
            omnia_wasi_vcs::store::merge(at, revision.to_owned(), message.to_owned(), policy)
                .await?;
        Ok(merged.into())
    }

    async fn log(&self, repo: &str, revision: &str, base: &str) -> Result<Vec<Entry>, Error> {
        let directories = wire::preopens();
        let repo = wire::lend(&directories, repo)?;
        let entries =
            omnia_wasi_vcs::store::log(repo, revision.to_owned(), base.to_owned()).await?;
        Ok(entries.into_iter().map(Into::into).collect())
    }

    async fn init(&self, at: &str) -> Result<(), Error> {
        let directories = wire::preopens();
        let at = wire::lend(&directories, at)?;
        Ok(omnia_wasi_vcs::workspace::init(at).await?)
    }

    async fn add(&self, repo: &str, at: &str, revision: &str) -> Result<(), Error> {
        let directories = wire::preopens();
        let repo = wire::lend(&directories, repo)?;
        let at = wire::lend(&directories, at)?;
        Ok(omnia_wasi_vcs::workspace::add(repo, at, revision.to_owned()).await?)
    }

    async fn remove(&self, at: &str) -> Result<(), Error> {
        let directories = wire::preopens();
        let at = wire::lend(&directories, at)?;
        Ok(omnia_wasi_vcs::workspace::remove(at).await?)
    }

    async fn pending(&self, at: &str) -> Result<Vec<Change>, Error> {
        let directories = wire::preopens();
        let at = wire::lend(&directories, at)?;
        let changes = omnia_wasi_vcs::workspace::pending(at).await?;
        Ok(changes.into_iter().map(Into::into).collect())
    }

    async fn clone_repo(&self, url: &str, at: &str, options: CloneOptions) -> Result<(), Error> {
        let directories = wire::preopens();
        let at = wire::lend(&directories, at)?;
        Ok(omnia_wasi_vcs::transport::clone(url.to_owned(), at, options.into()).await?)
    }

    async fn fetch(&self, repo: &str, remote: &str) -> Result<(), Error> {
        let directories = wire::preopens();
        let repo = wire::lend(&directories, repo)?;
        Ok(omnia_wasi_vcs::transport::fetch(repo, remote.to_owned()).await?)
    }

    async fn label(&self, repo: &str, name: &str, revision: &str) -> Result<(), Error> {
        let directories = wire::preopens();
        let repo = wire::lend(&directories, repo)?;
        Ok(omnia_wasi_vcs::transport::label(repo, name.to_owned(), revision.to_owned()).await?)
    }

    async fn labelled(&self, repo: &str, name: &str) -> Result<String, Error> {
        let directories = wire::preopens();
        let repo = wire::lend(&directories, repo)?;
        Ok(omnia_wasi_vcs::transport::labelled(repo, name.to_owned()).await?)
    }

    async fn fetched(&self, repo: &str, remote: &str, name: &str) -> Result<String, Error> {
        let directories = wire::preopens();
        let repo = wire::lend(&directories, repo)?;
        Ok(omnia_wasi_vcs::transport::fetched(repo, remote.to_owned(), name.to_owned()).await?)
    }

    async fn push(&self, repo: &str, remote: &str, label: &str) -> Result<(), Error> {
        let directories = wire::preopens();
        let repo = wire::lend(&directories, repo)?;
        Ok(omnia_wasi_vcs::transport::push(repo, remote.to_owned(), label.to_owned()).await?)
    }
}

// the lend helper
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
mod wire {
    use omnia_wasi_vcs::types;
    use wasip3::filesystem::preopens;
    use wasip3::filesystem::types::Descriptor;

    use super::Error;
    use crate::capabilities::lend::resolve_lend;

    // the lent root borrows a descriptor, so the preopens outlive the call
    pub fn preopens() -> Vec<(Descriptor, String)> {
        preopens::get_directories()
    }

    pub fn lend<'a>(
        directories: &'a [(Descriptor, String)], path: &str,
    ) -> Result<types::Location<'a>, Error> {
        let (root, subpath) = resolve_lend(directories, path)
            .ok_or_else(|| Error::Other(format!("location `{path}` matches no preopen")))?;
        Ok(types::Location {
            root,
            subpath: subpath.to_owned(),
        })
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
