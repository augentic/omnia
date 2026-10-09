//! # WASI VCS Service
//!
//! Host side of the `omnia:vcs` boundary. Follows the shared host-crate
//! shape (see `wasi-keyvalue`), with the `location` grant of `wasi-model`'s
//! workspace lend: every operation names its repository or working copy as
//! a borrowed mount-root descriptor plus a subpath, which the host resolves
//! against the store's mount registry into an open directory handle the
//! backend holds for as long as it runs.

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

pub use omnia_core::FutureResult;
use omnia_core::{HasMounts, Host, Server, StoreView};
use wasmtime::component::{Access, Accessor, HasData, Linker};

pub use self::generated::omnia::vcs::store::{Entry, Merged, Rule, Strategy};
pub use self::generated::omnia::vcs::transport::CloneOptions;
pub use self::generated::omnia::vcs::types::Error;
use self::generated::omnia::vcs::types::{Host as TypesHost, Location};
pub use self::generated::omnia::vcs::workspace::{Change, ChangeKind};
use self::generated::omnia::vcs::{store, transport, types, workspace};
use self::location::Intent;
pub use self::location::Place;

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
    // Resolve a guest location to the place the backend works through,
    // refusing a mutation beneath a read-only mount before any backend runs.
    fn locate<T: HasMounts>(
        access: &mut Access<'_, T, Self>, location: &Location, intent: Intent,
    ) -> Result<Place> {
        let mounts = access.data_mut().mounts();
        let descriptor = access.get().table.get(&location.root)?;
        location::resolve(descriptor, &mounts, location, intent).map_err(Self::lower)
    }

    // The locations resolve inside the store access, where the borrowed
    // descriptors are valid; the backend is awaited outside it, holding the
    // opened places rather than paths a guest could redirect meanwhile.
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
/// `omnia:vcs` operation, each over the [`Place`] the guest's location
/// resolved to. The backend never sees a descriptor and the guest never
/// sees a path. A place for `init`, `clone`, or the working copy `add`
/// lays down exists when the backend runs, empty where nothing stood
/// before; every other place must already exist, and one that does not is
/// the host's [`Error::NotARepository`] before the backend runs. A typed
/// failure is returned as an [`Error`] inside the `anyhow` error
/// (`Err(Error::NotFound(..).into())`) and reaches the guest as that
/// variant; any other error reaches it as [`Error::Other`].
///
/// The transport operation `clone` is `clone_repo` here, since `clone(&self,
/// ..)` would shadow [`Clone::clone`] on every backend that is `Clone`.
pub trait WasiVcsCtx: Debug + Send + Sync + 'static {
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

    /// Send `label` and the commits it reaches to `remote`.
    fn push(&self, repo: Place, remote: String, label: String) -> FutureResult<()>;
}

impl WasiVcsCtx for Box<dyn WasiVcsCtx> {
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

    fn push(&self, repo: Place, remote: String, label: String) -> FutureResult<()> {
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
