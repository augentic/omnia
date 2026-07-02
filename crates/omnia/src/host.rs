//! Traits implemented by WASI host crates (`wasi-*`).
//!
//! Each host crate provides a `WasiXxx` type implementing [`Host`] (and usually
//! [`Server`]) plus a default backend type implementing [`Backend`].

use std::fmt::Debug;
use std::future::Future;

use anyhow::Result;
use futures::future::BoxFuture;
use wasmtime::component::Linker;

use crate::runtime::Runtime;

/// Result type for asynchronous host operations.
pub type FutureResult<T> = BoxFuture<'static, Result<T>>;

/// Link a WASI host's generated bindings into the deployment linker.
pub trait Host<T>: Debug + Sync + Send {
    /// Link the host's dependencies prior to component instantiation.
    ///
    /// # Errors
    ///
    /// Returns an linking error(s) from the service's generated bindings.
    fn add_to_linker(linker: &mut Linker<T>) -> Result<()>;
}

/// Start a WASI host — typically a long-lived trigger server.
///
/// Parameterized by the deployment's backend bundle `B` so [`run`](Self::run)
/// receives the concrete [`Runtime<B>`].
pub trait Server<B>: Debug + Sync + Send {
    /// Whether this host is a long-lived trigger server — one whose
    /// [`run`](Self::run) loops on a transport and returns only on shutdown
    /// (e.g. `WasiHttp`, `WasiMessaging`, `WasiWebSocket`).
    ///
    /// Defaults to `false`: a capability host with the no-op [`run`](Self::run)
    /// (e.g. `WasiKeyValue`, `WasiBlobstore`, `WasiOtel`). The `runtime!` macro
    /// reads this flag from the *type system* — to select which hosts to `run`.
    const IS_SERVER: bool = false;

    /// Start the service.
    ///
    /// This is typically implemented by services that instantiate (or run)
    /// wasm components.
    #[allow(unused_variables)]
    fn run(&self, state: &Runtime<B>) -> impl Future<Output = Result<()>> {
        async { Ok(()) }
    }
}

/// Connect a host backend resource during runtime startup.
pub trait Backend: Sized + Sync + Send {
    /// The options used to connect to the backend.
    type ConnectOptions: FromEnv;

    /// Connect to the resource.
    #[must_use]
    fn connect() -> impl Future<Output = Result<Self>> {
        async { Self::connect_with(Self::ConnectOptions::from_env()?).await }
    }

    /// Connect with the specified options.
    fn connect_with(options: Self::ConnectOptions) -> impl Future<Output = Result<Self>>;
}

/// Create backend connection options from environment variables.
pub trait FromEnv: Sized {
    /// Create connection options from environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if required environment variables are missing or invalid.
    fn from_env() -> Result<Self>;
}
