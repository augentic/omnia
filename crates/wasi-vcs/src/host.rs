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

pub use omnia_backend::vcs::{
    Change, ChangeKind, CloneOptions, Entry, Error, Merged, Place, Rule, Strategy, VcsClient,
};
pub use omnia_core::FutureResult;
use omnia_core::{HasMounts, Host, Server, StoreView};
use wasmtime::component::{Access, Accessor, HasData, Linker};

pub use self::generated::omnia::vcs::types::Error as WitError;
use self::generated::omnia::vcs::types::{Host as TypesHost, Location};
use self::generated::omnia::vcs::{store, transport, types, workspace};
use self::location::Intent;

/// Result type for VCS operations.
pub type Result<T> = std::result::Result<T, WitError>;

/// Host-side service for `omnia:vcs`.
#[derive(Debug)]
pub struct WasiVcs;

impl HasData for WasiVcs {
    type Data<'a> = WasiVcsView<'a>;
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
    fn locate<T: HasMounts>(
        access: &mut Access<'_, T, Self>, location: &Location, intent: Intent,
    ) -> Result<Place> {
        let mounts = access.data_mut().mounts();
        let descriptor = access.get().table.get(&location.root)?;
        location::resolve(descriptor, &mounts, location, intent).map_err(Self::lower)
    }

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

    fn lower(error: anyhow::Error) -> WitError {
        to_wit_error(Error::from(error))
    }
}

fn to_wit_error(err: Error) -> WitError {
    match err {
        Error::NotARepository => WitError::NotARepository,
        Error::Exists(d) => WitError::Exists(d),
        Error::NotFound(d) => WitError::NotFound(d),
        Error::Pending(p) => WitError::Pending(p),
        Error::Access(d) => WitError::Access(d),
        Error::Diverged(d) => WitError::Diverged(d),
        Error::Other(d) => WitError::Other(d),
    }
}

// An untyped host failure is an `other` error at the boundary.
omnia_core::host_error!(WitError, Other);
omnia_core::wasi_view!(Vcs);

impl TypesHost for WasiVcsView<'_> {
    fn convert_error(&mut self, err: WitError) -> wasmtime::Result<WitError> {
        Ok(err)
    }
}
