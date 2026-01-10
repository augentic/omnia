//! # Traits for WASI Components
//!
//! This module contains traits implemented by concrete WASI services.
//!
//! Each service is a module that provides a concrete implementation in support
//! of a specific set of WASI interfaces.

use std::fmt::Debug;

use anyhow::Result;
use futures::future::BoxFuture;
use wasmtime::component::{InstancePre, Linker};
use wasmtime_wasi::WasiView;

pub type FutureResult<T> = BoxFuture<'static, Result<T>>;

pub trait State: Clone + Send + Sync + 'static {
    type StoreCtx: Send;

    #[must_use]
    fn store(&self) -> Self::StoreCtx;

    fn instance_pre(&self) -> &InstancePre<Self::StoreCtx>;
}

/// Implemented by all WASI hosts in order to allow the runtime to link their
/// dependencies.
pub trait Host<T>: Debug + Sync + Send {
    /// Link the host's dependencies prior to component instantiation.
    ///
    /// # Errors
    ///
    /// Returns an linking error(s) from the service's generated bindings.
    fn add_to_linker(linker: &mut Linker<T>) -> Result<()>;
}

use wasmtime::component::HasData;

pub trait View<'a, T: WasiView>: HasData + 'a + Send {
    // type Data<'a>: HasData + 'a;

    fn view(ctx: &mut T) -> <Self as HasData>::Data<'_>;
}

/// Implemented by WASI hosts that are servers in order to allow the runtime to
/// start them.
pub trait Server<S: State>: Debug + Sync + Send {
    /// Start the service.
    ///
    /// This is typically implemented by services that instantiate (or run)
    /// wasm components.
    #[allow(unused_variables)]
    fn run(&self, state: &S) -> impl Future<Output = Result<()>> {
        async { Ok(()) }
    }
}

/// Implemented by backend resources to allow the backend to be connected to a
/// WASI component.
pub trait Backend: Sized + Sync + Send {
    type ConnectOptions: FromEnv;

    /// Connect to the resource.
    #[must_use]
    fn connect() -> impl Future<Output = Result<Self>> {
        async { Self::connect_with(Self::ConnectOptions::from_env()?).await }
    }

    fn connect_with(options: Self::ConnectOptions) -> impl Future<Output = Result<Self>>;
}

pub trait FromEnv: Sized {
    /// Create connection options from environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if required environment variables are missing or invalid.
    fn from_env() -> Result<Self>;
}
