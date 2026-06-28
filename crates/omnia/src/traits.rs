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
use wasmtime::component::{Instance, InstancePre, Linker};
use wasmtime::{Store, StoreLimits};
use wasmtime_wasi::WasiView;
use wrpc_wasmtime::WrpcView;

use crate::RuntimeOptions;
use crate::registry::Registry;

/// Result type for asynchronous operations.
pub type FutureResult<T> = BoxFuture<'static, Result<T>>;

/// Exposes a store context's [`StoreLimits`] so the runtime can install a
/// per-guest resource limiter on every [`Store`] it creates.
pub trait HasLimits {
    /// Returns a mutable reference to the context's resource limits.
    fn limits(&mut self) -> &mut StoreLimits;
}

/// The long-lived, `Clone` host runtime context every trigger server is handed
/// to resolve and instantiate a guest.
///
/// It owns the [`Registry`], the runtime options, and the per-call
/// instantiation helpers. The per-store state is its [`Runtime::StoreCtx`]
/// associated type — not this trait.
pub trait Runtime: Clone + Send + Sync + 'static {
    /// The store context type.
    type StoreCtx: WasiView + WrpcView + 'static + Send + HasLimits;

    /// Returns the store context.
    #[must_use]
    fn store(&self) -> Self::StoreCtx;

    /// Returns the multi-guest registry.
    fn registry(&self) -> &Registry<Self::StoreCtx>;

    /// Returns the environment-derived runtime options.
    ///
    /// Defaults to the registry's options, which every runtime already owns; an
    /// implementation only overrides this if it sources options elsewhere.
    fn options(&self) -> &RuntimeOptions {
        self.registry().options()
    }

    /// Build a fully configured [`Store`] for a single guest invocation.
    ///
    /// Installs an epoch deadline (so CPU-bound guests periodically yield to
    /// the async executor, allowing an enclosing wall-clock timeout to fire),
    /// an optional fuel budget, and the per-guest memory limiter.
    #[must_use]
    fn build_store(&self, data: Self::StoreCtx) -> Store<Self::StoreCtx> {
        let options = self.options();
        let mut store = Store::new(self.registry().engine(), data);

        // Yield to the executor every epoch tick; the deadline is bumped on each
        // yield so execution continues until a surrounding `timeout` cancels it.
        store.set_epoch_deadline(1);
        store.epoch_deadline_async_yield_and_update(1);

        if options.max_fuel > 0 {
            // `consume_fuel` is enabled in `compile_config` whenever a budget is
            // set, so this only fails on a compile/run configuration mismatch.
            if let Err(error) = store.set_fuel(options.max_fuel) {
                tracing::warn!(%error, "failed to set fuel budget");
            }
        }

        store.limiter(|ctx| ctx.limits());
        store
    }

    /// Instantiate a selected guest's pre-instantiated component into `store`,
    /// recording instantiation latency (the `instantiation_duration_us`
    /// histogram) and failures (the `pool_instantiation_errors` counter, a proxy
    /// for pool exhaustion) as `OpenTelemetry` metrics.
    ///
    /// The caller passes the [`InstancePre`] resolved from the registry (the
    /// default guest, or an identity-selected one) so a dispatched call lands in
    /// a fresh instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the component cannot be instantiated, e.g. when the
    /// pooling allocator is exhausted.
    fn instantiate(
        &self, instance_pre: &InstancePre<Self::StoreCtx>, store: &mut Store<Self::StoreCtx>,
    ) -> impl Future<Output = Result<Instance>> + Send {
        async move {
            match instance_pre.instantiate_async(store).await {
                Ok(instance) => {
                    tracing::debug!("component instantiated");
                    Ok(instance)
                }
                Err(error) => Err(error.into()),
            }
        }
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
pub trait Server<R: Runtime>: Debug + Sync + Send {
    /// Whether this host is a long-lived trigger server — one whose
    /// [`run`](Self::run) loops on a transport and returns only on shutdown
    /// (e.g. `WasiHttp`, `WasiMessaging`, `WasiWebSocket`).
    ///
    /// Defaults to `false`: a capability host with the no-op [`run`](Self::run)
    /// (e.g. `WasiKeyValue`, `WasiBlobstore`, `WasiOtel`). The `runtime!` macro
    /// reads this flag from the *type system* to skip linking long-lived triggers
    /// in a `command` deployment, so a newly added trigger is covered without
    /// editing the macro.
    const IS_SERVER: bool = false;

    /// Start the service.
    ///
    /// This is typically implemented by services that instantiate (or run)
    /// wasm components.
    #[allow(unused_variables)]
    fn run(&self, state: &R) -> impl Future<Output = Result<()>> {
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

#[cfg(test)]
mod tests {
    /// Compile-time guard that a runtime links no long-lived trigger server in
    /// command mode. The `runtime!` macro enforces this by skipping
    /// [`Server::IS_SERVER`] hosts when linking instead.
    const fn assert_command_hosts(hosts: &[bool]) {
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

    #[test]
    fn capability_only_is_allowed() {
        // A command deployment with only capability hosts (every `IS_SERVER`
        // false) is fine.
        assert_command_hosts(&[false, false, false]);
    }

    #[test]
    fn empty_is_allowed() {
        assert_command_hosts(&[]);
    }

    #[test]
    #[should_panic(expected = "long-lived trigger server")]
    fn long_lived_server_is_rejected() {
        // Any `true` (a long-lived trigger) in a command deployment fails.
        assert_command_hosts(&[false, true]);
    }
}
