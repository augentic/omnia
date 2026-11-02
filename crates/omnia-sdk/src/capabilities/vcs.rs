//! Version-control capability over `omnia:vcs`.
//!
//! Target-independent mirrors of the `omnia:vcs` records, and the [`Vcs`]
//! trait over them. The one record that cannot cross off `wasm32` is the
//! `location` — a borrowed `wasi:filesystem` descriptor plus a subpath — so
//! a guest names every repository and working copy with a deployment-local
//! path (`"."` the project mount, `"./.cache/repo"` or `"/mount/sub"`
//! beneath one) and the `wasm32` default bodies resolve it against the
//! guest's preopens at the call site, by the lend rule [`Model`] uses for
//! its workspace.
//!
//! [`Model`]: crate::Model

use std::future::Future;

pub use omnia_backend::vcs::{
    Change, ChangeKind, CloneOptions, Entry, Error, Merged, Rule, Strategy,
};

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

/// Version control (Omnia VCS).
///
/// Every location is a deployment-local path resolved against the guest's
/// preopens on `wasm32`: `"."` names the project mount, `"./sub"` or
/// `"/mount/sub"` a directory beneath one, which need not exist yet where
/// the operation creates it. Default WASM implementations delegate to
/// `omnia:vcs` via `omnia-wasi-vcs`; off `wasm32` the signatures are bare
/// so hosts and tests supply their own provider.
///
/// The transport operation `clone` is [`clone_repo`](Self::clone_repo)
/// here, since `clone(&self, ..)` would shadow [`Clone::clone`] on every
/// provider that is `Clone`.
pub trait Vcs: Send + Sync {
    cfg_select! {
        not(target_arch = "wasm32") => {
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
            fn fetch(
                &self, repo: &str, remote: &str,
            ) -> impl Future<Output = Result<(), Error>> + Send;

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
        _ => {
            /// The commit `revision` names in the repository at `repo`: a
            /// label, a tag, or a commit id, whole or a unique prefix.
            fn resolve(
                &self, repo: &str, revision: &str,
            ) -> impl Future<Output = Result<String, Error>> + Send {
                let (repo, revision) = (repo.to_owned(), revision.to_owned());
                async move {
                    let directories = wire::preopens();
                    let repo = wire::lend(&directories, &repo)?;
                    Ok(omnia_wasi_vcs::store::resolve(repo, revision).await?)
                }
            }

            /// Whether `ancestor` is in `descendant`'s history, itself included,
            /// in the repository at `repo`.
            fn descends(
                &self, repo: &str, ancestor: &str, descendant: &str,
            ) -> impl Future<Output = Result<bool, Error>> + Send {
                let (repo, ancestor, descendant) =
                    (repo.to_owned(), ancestor.to_owned(), descendant.to_owned());
                async move {
                    let directories = wire::preopens();
                    let repo = wire::lend(&directories, &repo)?;
                    Ok(omnia_wasi_vcs::store::descends(repo, ancestor, descendant).await?)
                }
            }

            /// The sealed commit the working copy at `at` sits on; one with
            /// no commit yet is [`Error::NotFound`].
            fn head(&self, at: &str) -> impl Future<Output = Result<String, Error>> + Send {
                let at = at.to_owned();
                async move {
                    let directories = wire::preopens();
                    let at = wire::lend(&directories, &at)?;
                    Ok(omnia_wasi_vcs::store::head(at).await?)
                }
            }

            /// Seals every pending change at `at` as one commit; `None` when
            /// there is nothing to seal.
            fn commit(
                &self, at: &str, message: &str,
            ) -> impl Future<Output = Result<Option<String>, Error>> + Send {
                let (at, message) = (at.to_owned(), message.to_owned());
                async move {
                    let directories = wire::preopens();
                    let at = wire::lend(&directories, &at)?;
                    Ok(omnia_wasi_vcs::store::commit(at, message).await?)
                }
            }

            /// Merges `revision` into the working copy at `at` under `policy`
            /// and advances its head.
            fn merge(
                &self, at: &str, revision: &str, message: &str, policy: &[Rule],
            ) -> impl Future<Output = Result<Merged, Error>> + Send {
                let (at, revision, message) =
                    (at.to_owned(), revision.to_owned(), message.to_owned());
                let policy: Vec<omnia_wasi_vcs::store::Rule> =
                    policy.iter().cloned().map(Into::into).collect();
                async move {
                    let directories = wire::preopens();
                    let at = wire::lend(&directories, &at)?;
                    let merged =
                        omnia_wasi_vcs::store::merge(at, revision, message, policy).await?;
                    Ok(merged.into())
                }
            }

            /// The first-parent chain from `revision` back to `base` in the
            /// repository at `repo`, newest first, `base` left out.
            fn log(
                &self, repo: &str, revision: &str, base: &str,
            ) -> impl Future<Output = Result<Vec<Entry>, Error>> + Send {
                let (repo, revision, base) =
                    (repo.to_owned(), revision.to_owned(), base.to_owned());
                async move {
                    let directories = wire::preopens();
                    let repo = wire::lend(&directories, &repo)?;
                    let entries = omnia_wasi_vcs::store::log(repo, revision, base).await?;
                    Ok(entries.into_iter().map(Into::into).collect())
                }
            }

            /// A repository with no history at `at`.
            fn init(&self, at: &str) -> impl Future<Output = Result<(), Error>> + Send {
                let at = at.to_owned();
                async move {
                    let directories = wire::preopens();
                    let at = wire::lend(&directories, &at)?;
                    Ok(omnia_wasi_vcs::workspace::init(at).await?)
                }
            }

            /// A working copy of `repo` at `at`, on `revision` and on no label.
            fn add(
                &self, repo: &str, at: &str, revision: &str,
            ) -> impl Future<Output = Result<(), Error>> + Send {
                let (repo, at, revision) = (repo.to_owned(), at.to_owned(), revision.to_owned());
                async move {
                    let directories = wire::preopens();
                    let repo = wire::lend(&directories, &repo)?;
                    let at = wire::lend(&directories, &at)?;
                    Ok(omnia_wasi_vcs::workspace::add(repo, at, revision).await?)
                }
            }

            /// Removes the working copy at `at` and its files; pending changes
            /// go with it.
            fn remove(&self, at: &str) -> impl Future<Output = Result<(), Error>> + Send {
                let at = at.to_owned();
                async move {
                    let directories = wire::preopens();
                    let at = wire::lend(&directories, &at)?;
                    Ok(omnia_wasi_vcs::workspace::remove(at).await?)
                }
            }

            /// What the working copy at `at` holds that its head does not.
            fn pending(&self, at: &str) -> impl Future<Output = Result<Vec<Change>, Error>> + Send {
                let at = at.to_owned();
                async move {
                    let directories = wire::preopens();
                    let at = wire::lend(&directories, &at)?;
                    let changes = omnia_wasi_vcs::workspace::pending(at).await?;
                    Ok(changes.into_iter().map(Into::into).collect())
                }
            }

            /// A clone of `url` at `at`, which must hold nothing yet.
            fn clone_repo(
                &self, url: &str, at: &str, options: CloneOptions,
            ) -> impl Future<Output = Result<(), Error>> + Send {
                let (url, at) = (url.to_owned(), at.to_owned());
                async move {
                    let directories = wire::preopens();
                    let at = wire::lend(&directories, &at)?;
                    Ok(omnia_wasi_vcs::transport::clone(url, at, options.into()).await?)
                }
            }

            /// Brings `remote`'s commits and labels into the repository at `repo`.
            fn fetch(
                &self, repo: &str, remote: &str,
            ) -> impl Future<Output = Result<(), Error>> + Send {
                let (repo, remote) = (repo.to_owned(), remote.to_owned());
                async move {
                    let directories = wire::preopens();
                    let repo = wire::lend(&directories, &repo)?;
                    Ok(omnia_wasi_vcs::transport::fetch(repo, remote).await?)
                }
            }

            /// Points `name` at `revision` in the repository at `repo`, creating
            /// or moving it.
            fn label(
                &self, repo: &str, name: &str, revision: &str,
            ) -> impl Future<Output = Result<(), Error>> + Send {
                let (repo, name, revision) =
                    (repo.to_owned(), name.to_owned(), revision.to_owned());
                async move {
                    let directories = wire::preopens();
                    let repo = wire::lend(&directories, &repo)?;
                    Ok(omnia_wasi_vcs::transport::label(repo, name, revision).await?)
                }
            }

            /// The commit the label `name` points at in the repository at
            /// `repo`, in the namespace [`label`](Self::label) writes alone.
            fn labelled(
                &self, repo: &str, name: &str,
            ) -> impl Future<Output = Result<String, Error>> + Send {
                let (repo, name) = (repo.to_owned(), name.to_owned());
                async move {
                    let directories = wire::preopens();
                    let repo = wire::lend(&directories, &repo)?;
                    Ok(omnia_wasi_vcs::transport::labelled(repo, name).await?)
                }
            }

            /// The commit `remote`'s label `name` pointed at when the
            /// repository at `repo` last fetched from it, in the namespace
            /// [`fetch`](Self::fetch) and [`clone_repo`](Self::clone_repo)
            /// write alone.
            fn fetched(
                &self, repo: &str, remote: &str, name: &str,
            ) -> impl Future<Output = Result<String, Error>> + Send {
                let (repo, remote, name) = (repo.to_owned(), remote.to_owned(), name.to_owned());
                async move {
                    let directories = wire::preopens();
                    let repo = wire::lend(&directories, &repo)?;
                    Ok(omnia_wasi_vcs::transport::fetched(repo, remote, name).await?)
                }
            }

            /// Sends `label` and the commits it reaches to `remote`; nothing is
            /// forced.
            fn push(
                &self, repo: &str, remote: &str, label: &str,
            ) -> impl Future<Output = Result<(), Error>> + Send {
                let (repo, remote, label) = (repo.to_owned(), remote.to_owned(), label.to_owned());
                async move {
                    let directories = wire::preopens();
                    let repo = wire::lend(&directories, &repo)?;
                    Ok(omnia_wasi_vcs::transport::push(repo, remote, label).await?)
                }
            }
        }
    }
}

delegate_deref!(Vcs {
    fn resolve(
        &self, repo: &str, revision: &str,
    ) -> impl Future<Output = Result<String, Error>> + Send {
        (**self).resolve(repo, revision)
    }

    fn descends(
        &self, repo: &str, ancestor: &str, descendant: &str,
    ) -> impl Future<Output = Result<bool, Error>> + Send {
        (**self).descends(repo, ancestor, descendant)
    }

    fn head(&self, at: &str) -> impl Future<Output = Result<String, Error>> + Send {
        (**self).head(at)
    }

    fn commit(
        &self, at: &str, message: &str,
    ) -> impl Future<Output = Result<Option<String>, Error>> + Send {
        (**self).commit(at, message)
    }

    fn merge(
        &self, at: &str, revision: &str, message: &str, policy: &[Rule],
    ) -> impl Future<Output = Result<Merged, Error>> + Send {
        (**self).merge(at, revision, message, policy)
    }

    fn log(
        &self, repo: &str, revision: &str, base: &str,
    ) -> impl Future<Output = Result<Vec<Entry>, Error>> + Send {
        (**self).log(repo, revision, base)
    }

    fn init(&self, at: &str) -> impl Future<Output = Result<(), Error>> + Send {
        (**self).init(at)
    }

    fn add(
        &self, repo: &str, at: &str, revision: &str,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        (**self).add(repo, at, revision)
    }

    fn remove(&self, at: &str) -> impl Future<Output = Result<(), Error>> + Send {
        (**self).remove(at)
    }

    fn pending(&self, at: &str) -> impl Future<Output = Result<Vec<Change>, Error>> + Send {
        (**self).pending(at)
    }

    fn clone_repo(
        &self, url: &str, at: &str, options: CloneOptions,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        (**self).clone_repo(url, at, options)
    }

    fn fetch(&self, repo: &str, remote: &str) -> impl Future<Output = Result<(), Error>> + Send {
        (**self).fetch(repo, remote)
    }

    fn label(
        &self, repo: &str, name: &str, revision: &str,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        (**self).label(repo, name, revision)
    }

    fn labelled(&self, repo: &str, name: &str) -> impl Future<Output = Result<String, Error>> + Send {
        (**self).labelled(repo, name)
    }

    fn fetched(
        &self, repo: &str, remote: &str, name: &str,
    ) -> impl Future<Output = Result<String, Error>> + Send {
        (**self).fetched(repo, remote, name)
    }

    fn push(
        &self, repo: &str, remote: &str, label: &str,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        (**self).push(repo, remote, label)
    }
});

/// The WASI-backed provider a `wasm32` guest hands its wasm-free core; the
/// default method bodies carry the whole delegation.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug)]
pub struct WasiVcs;

#[cfg(target_arch = "wasm32")]
impl Vcs for WasiVcs {}

// the lend helper
#[cfg(target_arch = "wasm32")]
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

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn taxonomy_mapping() {
        let cases = [
            (Error::NotARepository, "not-a-repository"),
            (Error::Exists("x".into()), "exists"),
            (Error::NotFound("v1".into()), "not-found"),
            (Error::Pending(vec!["a".into(), "b".into()]), "pending"),
            (Error::Access("denied".into()), "access"),
            (Error::Diverged("emery/x".into()), "diverged"),
            (Error::Other("boom".into()), "other"),
        ];
        for (error, code) in cases {
            let mapped = crate::Error::from(error.clone());
            assert_eq!(mapped.code(), code);
            match error {
                Error::NotFound(_) => assert!(matches!(mapped, crate::Error::NotFound { .. })),
                Error::Access(_) => assert!(matches!(mapped, crate::Error::BadGateway { .. })),
                Error::Other(_) => assert!(matches!(mapped, crate::Error::ServerError { .. })),
                Error::NotARepository
                | Error::Exists(_)
                | Error::Pending(_)
                | Error::Diverged(_) => {
                    assert!(matches!(mapped, crate::Error::BadRequest { .. }));
                }
            }
        }
    }
}
