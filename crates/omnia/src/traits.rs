//! # Traits for WASI Components
//!
//! This module contains traits implemented by concrete WASI services.
//!
//! Each service is a module that provides a concrete implementation in support
//! of a specific set of WASI interfaces.

use std::fmt::Debug;
use std::future::Future;

use anyhow::Result;
use futures::future::BoxFuture;
use wasmtime::StoreLimits;
use wasmtime::component::Linker;

use crate::runtime::Runtime;

/// Result type for asynchronous operations.
pub type FutureResult<T> = BoxFuture<'static, Result<T>>;

/// Exposes a store context's [`StoreLimits`] so the runtime can install a
/// per-guest resource limiter on every [`Store`](wasmtime::Store) it creates.
pub trait HasLimits {
    /// Returns a mutable reference to the context's resource limits.
    fn limits(&mut self) -> &mut StoreLimits;
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

/// Implemented by WASI hosts that are servers in order to allow the runtime to
/// start them.
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

/// Compile-time guard that a `command: true` deployment includes no long-lived
/// trigger server.
///
/// The `runtime!` macro emits
/// `const _: () = omnia::assert_hosts(&[<Host as Server<_>>::IS_SERVER, …]);`
/// for a command deployment, so listing a trigger host (`WasiHttp`,
/// `WasiMessaging`, `WasiWebSocket`) is a build error rather than a silently
/// dropped host. The values come straight from [`Server::IS_SERVER`], so a newly
/// added trigger is covered without editing the macro.
///
/// # Panics
///
/// Panics if any element is `true` (a host is a long-lived trigger server). In
/// the macro's const context this surfaces as a compile error.
#[doc(hidden)]
pub const fn assert_hosts(hosts: &[bool]) {
    let mut index = 0;
    while index < hosts.len() {
        assert!(
            !hosts[index],
            "a `command: true` deployment cannot link a long-lived trigger server (`WasiHttp`, \
             `WasiMessaging`, `WasiWebSocket`): a command runs to completion and exits, but the \
             server would run forever. Use the default `command: false` for a server deployment, \
             or drop the trigger host — capability hosts (`WasiKeyValue`, `WasiBlobstore`, ...) \
             are fine to link."
        );
        index += 1;
    }
}

/// Implemented by backend resources to allow the backend to be connected to a
/// WASI component.
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

/// Trait for creating connection options from environment variables.
pub trait FromEnv: Sized {
    /// Create connection options from environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if required environment variables are missing or invalid.
    fn from_env() -> Result<Self>;
}

/// A deployment's connected backend bundle, threaded into [`Runtime`].
///
/// The `runtime!` macro generates the concrete bundle (one field per declared
/// backend) and this impl, whose [`connect`](Self::connect) connects every
/// backend concurrently — the work the macro previously inlined as a
/// `tokio::try_join!` in the generated `Runtime::new`. A deployment that wires
/// no backends uses the [`()`](unit) bundle below, so [`Runtime`] needs no
/// special empty case.
///
/// [`Runtime`]: crate::Runtime
pub trait Backends: Clone + Send + Sync + 'static {
    /// Connect every backend in the bundle.
    ///
    /// # Errors
    ///
    /// Returns the first backend connection error.
    fn connect() -> impl Future<Output = Result<Self>>;
}

/// The zero-backend bundle: a deployment that links only backend-less hosts
/// (such as a `command: true` `wasi:cli` deployment) connects nothing.
impl Backends for () {
    async fn connect() -> Result<Self> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::assert_hosts;

    #[test]
    fn capability_only_is_allowed() {
        // A command deployment with only capability hosts (every `IS_SERVER`
        // false) is fine.
        assert_hosts(&[false, false, false]);
    }

    #[test]
    fn empty_is_allowed() {
        assert_hosts(&[]);
    }

    #[test]
    #[should_panic(expected = "long-lived trigger server")]
    fn long_lived_server_is_rejected() {
        // Any `true` (a long-lived trigger) in a command deployment fails.
        assert_hosts(&[false, true]);
    }
}
