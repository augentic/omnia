//! # WASI VCS Service
//!
//! Host side of the `omnia:vcs` boundary. Follows the shared host-crate
//! shape (see `wasi-keyvalue`), with the `location` grant of `wasi-model`'s
//! workspace lend: every operation names its repository or working copy as
//! a borrowed mount-root descriptor plus a subpath, which the host resolves
//! against the store's mount registry before the backend runs.

mod location;
mod store_impl;
mod transport_impl;
mod workspace_impl;

mod generated {
    #![allow(missing_docs, reason = "wasmtime bindgen output")]

    pub use self::omnia::vcs::types::Error;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "wasi:clocks": wasmtime_wasi::p3::bindings::clocks,
            "wasi:filesystem": wasmtime_wasi::p3::bindings::filesystem,
        },
        trappable_error_type: {
            "omnia:vcs/types.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::path::PathBuf;

pub use omnia_core::FutureResult;
use omnia_core::{HasMounts, Host, Server, StoreView};
use wasmtime::component::{Access, Accessor, HasData, Linker};

pub use self::generated::omnia::vcs::store::{Merged, Rule, Strategy};
pub use self::generated::omnia::vcs::transport::CloneOptions;
pub use self::generated::omnia::vcs::types::Error;
use self::generated::omnia::vcs::types::{Host as TypesHost, Location};
pub use self::generated::omnia::vcs::workspace::{Change, ChangeKind};
use self::generated::omnia::vcs::{store, transport, types, workspace};
use self::location::Intent;

/// Result type for VCS operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Host-side service for `omnia:vcs`.
#[derive(Debug)]
pub struct WasiVcs;

impl HasData for WasiVcs {
    type Data<'a> = WasiVcsCtxView<'a>;
}

impl<T> Host<T> for WasiVcs
where
    T: StoreView<Self> + HasMounts + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        types::add_to_linker::<_, Self>(linker, T::view)?;
        store::add_to_linker::<_, Self>(linker, T::view)?;
        workspace::add_to_linker::<_, Self>(linker, T::view)?;
        Ok(transport::add_to_linker::<_, Self>(linker, T::view)?)
    }
}

impl<B> Server<B> for WasiVcs {}

impl WasiVcs {
    // Resolve a guest location to the host path the backend works on,
    // refusing a mutation beneath a read-only mount before any backend runs.
    fn locate<T: HasMounts>(
        access: &mut Access<'_, T, Self>, location: &Location, intent: Intent,
    ) -> Result<PathBuf> {
        let mounts = access.data_mut().mounts();
        let descriptor = access.get().table.get(&location.root)?;
        Ok(location::resolve(descriptor, &mounts, location, intent)?)
    }

    // The locations resolve inside the store access, where the borrowed
    // descriptors are valid; the backend is awaited outside it.
    async fn dispatch<T, R>(
        accessor: &Accessor<T, Self>,
        op: impl FnOnce(&mut Access<'_, T, Self>) -> Result<FutureResult<R>>,
    ) -> Result<R>
    where
        T: HasMounts,
    {
        let pending = accessor.with(|mut access| op(&mut access))?;
        pending.await.map_err(Self::lower)
    }

    // A backend's typed failure passes through; anything else is `other`.
    fn lower(error: anyhow::Error) -> Error {
        error.downcast::<Error>().unwrap_or_else(Into::into)
    }
}

/// A trait which provides internal WASI VCS context.
///
/// This is implemented by the version-control backend: one method per
/// `omnia:vcs` operation, each over the resolved host path of the location
/// the guest named. The backend never sees a descriptor and the guest never
/// sees a path. A typed failure is returned as an [`Error`] inside the
/// `anyhow` error (`Err(Error::NotFound(..).into())`) and reaches the guest
/// as that variant; any other error reaches it as [`Error::Other`].
///
/// The transport operation `clone` is `clone_repo` here, since `clone(&self,
/// ..)` would shadow [`Clone::clone`] on every backend that is `Clone`.
pub trait WasiVcsCtx: Debug + Send + Sync + 'static {
    /// The commit `revision` names in the repository at `repo`.
    fn resolve(&self, repo: PathBuf, revision: String) -> FutureResult<String>;

    /// The sealed commit the working copy at `at` sits on.
    fn head(&self, at: PathBuf) -> FutureResult<String>;

    /// Seal every pending change at `at` as one commit; `None` when there
    /// is nothing to seal.
    fn commit(&self, at: PathBuf, message: String) -> FutureResult<Option<String>>;

    /// Merge `revision` into the working copy at `at` under `policy`.
    fn merge(
        &self, at: PathBuf, revision: String, message: String, policy: Vec<Rule>,
    ) -> FutureResult<Merged>;

    /// A repository with no history at `at`.
    fn init(&self, at: PathBuf) -> FutureResult<()>;

    /// A working copy of `repo` at `at`, detached at `revision`.
    fn add(&self, repo: PathBuf, at: PathBuf, revision: String) -> FutureResult<()>;

    /// Remove the working copy at `at` and its files.
    fn remove(&self, at: PathBuf) -> FutureResult<()>;

    /// What the working copy at `at` holds that its head does not.
    fn pending(&self, at: PathBuf) -> FutureResult<Vec<Change>>;

    /// A clone of `url` at `at`.
    fn clone_repo(&self, url: String, at: PathBuf, options: CloneOptions) -> FutureResult<()>;

    /// Bring `remote`'s commits and labels into the repository at `repo`.
    fn fetch(&self, repo: PathBuf, remote: String) -> FutureResult<()>;

    /// Point `name` at `revision` in the repository at `repo`.
    fn label(&self, repo: PathBuf, name: String, revision: String) -> FutureResult<()>;

    /// Send `label` and the commits it reaches to `remote`.
    fn push(&self, repo: PathBuf, remote: String, label: String) -> FutureResult<()>;
}

impl WasiVcsCtx for Box<dyn WasiVcsCtx> {
    fn resolve(&self, repo: PathBuf, revision: String) -> FutureResult<String> {
        (**self).resolve(repo, revision)
    }

    fn head(&self, at: PathBuf) -> FutureResult<String> {
        (**self).head(at)
    }

    fn commit(&self, at: PathBuf, message: String) -> FutureResult<Option<String>> {
        (**self).commit(at, message)
    }

    fn merge(
        &self, at: PathBuf, revision: String, message: String, policy: Vec<Rule>,
    ) -> FutureResult<Merged> {
        (**self).merge(at, revision, message, policy)
    }

    fn init(&self, at: PathBuf) -> FutureResult<()> {
        (**self).init(at)
    }

    fn add(&self, repo: PathBuf, at: PathBuf, revision: String) -> FutureResult<()> {
        (**self).add(repo, at, revision)
    }

    fn remove(&self, at: PathBuf) -> FutureResult<()> {
        (**self).remove(at)
    }

    fn pending(&self, at: PathBuf) -> FutureResult<Vec<Change>> {
        (**self).pending(at)
    }

    fn clone_repo(&self, url: String, at: PathBuf, options: CloneOptions) -> FutureResult<()> {
        (**self).clone_repo(url, at, options)
    }

    fn fetch(&self, repo: PathBuf, remote: String) -> FutureResult<()> {
        (**self).fetch(repo, remote)
    }

    fn label(&self, repo: PathBuf, name: String, revision: String) -> FutureResult<()> {
        (**self).label(repo, name, revision)
    }

    fn push(&self, repo: PathBuf, remote: String, label: String) -> FutureResult<()> {
        (**self).push(repo, remote, label)
    }
}

impl TypesHost for WasiVcsCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

// An untyped host failure is an `other` error at the boundary.
omnia_core::host_error!(Error, Other);
omnia_core::wasi_view!(Vcs);
