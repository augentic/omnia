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
use wasmtime::component::{InstancePre, Linker};
use wasmtime::{Store, StoreLimits};

use crate::RuntimeConfig;

/// Result type for asynchronous operations.
pub type FutureResult<T> = BoxFuture<'static, Result<T>>;

/// Exposes a store context's [`StoreLimits`] so the runtime can install a
/// per-guest resource limiter on every [`Store`] it creates.
pub trait HasLimits {
    /// Returns a mutable reference to the context's resource limits.
    fn limits(&mut self) -> &mut StoreLimits;
}

/// State trait for WASI components.
pub trait State: Clone + Send + Sync + 'static {
    /// The store context type.
    type StoreCtx: Send + HasLimits;

    /// Returns the store context.
    #[must_use]
    fn store(&self) -> Self::StoreCtx;

    /// Returns the pre-instantiated component.
    fn instance_pre(&self) -> &InstancePre<Self::StoreCtx>;

    /// Returns the environment-derived runtime configuration.
    fn config(&self) -> &RuntimeConfig;

    /// Build a fully configured [`Store`] for a single guest invocation.
    ///
    /// Installs an epoch deadline (so CPU-bound guests periodically yield to
    /// the async executor, allowing an enclosing wall-clock timeout to fire),
    /// an optional fuel budget, and the per-guest memory limiter.
    #[must_use]
    fn new_store(&self, data: Self::StoreCtx) -> Store<Self::StoreCtx> {
        let config = self.config();
        let mut store = Store::new(self.instance_pre().engine(), data);

        // Yield to the executor every epoch tick; the deadline is bumped on each
        // yield so execution continues until a surrounding `timeout` cancels it.
        store.set_epoch_deadline(1);
        store.epoch_deadline_async_yield_and_update(1);

        if config.max_fuel > 0 {
            // `consume_fuel` is enabled in `compile_config` whenever a budget is
            // set, so this only fails on a compile/run configuration mismatch.
            if let Err(error) = store.set_fuel(config.max_fuel) {
                tracing::warn!(%error, "failed to set fuel budget");
            }
        }

        store.limiter(|ctx| ctx.limits());
        store
    }
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
    /// The options used to connect to the backend.
    type ConnectOptions: FromEnv;

    /// Connect to the resource.
    #[must_use]
    fn connect() -> impl Future<Output = Result<Self>> {
        async { Self::connect_with(Self::ConnectOptions::from_env()?).await }
    }

    /// Connect to the resource with the specified options.
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
