//! Per-store context. [`StoreBase`] holds the state identical for every
//! deployment (WASI table/context, memory limiter, wRPC view state, host→guest
//! dispatcher); [`StoreCtx`] pairs it with the deployment's backend bundle `B`
//! and implements the fixed `WasiView`/`WrpcView`/`HasLimits` views, while each
//! host crate blankets its own `WasiXxxView for StoreCtx<B> where B: HasXxx`.

use std::sync::Arc;

use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{FsPerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtxView, WasiHttpView};
use wrpc_wasmtime::{WrpcCtxView, WrpcView};

use crate::{Dispatcher, LinkClient, MountRegistry, PluginLoader, RuntimeOptions, WrpcState};

/// Exposes a store context's [`StoreLimits`] so the runtime can install a
/// per-guest resource limiter on every [`Store`](wasmtime::Store) it creates.
pub trait HasLimits {
    /// Returns a mutable reference to the context's resource limits.
    fn limits(&mut self) -> &mut StoreLimits;
}

/// The per-store construction inputs for [`StoreBase::new`].
///
/// `options` and `dispatcher` are required; the rest default sensibly (empty
/// argv, no mounts, host env inheritance) so hand-written test runtimes build
/// unchanged.
pub struct StoreConfig<'a> {
    /// Runtime options; caps linear-memory growth at
    /// [`RuntimeOptions::max_memory_bytes`].
    pub options: &'a RuntimeOptions,
    /// Type-erased host->guest dispatcher: a fresh handle to the owning
    /// [`Runtime`](crate::Runtime) so any host->guest call (such as
    /// `wasi-model`'s `resolve`) lands a new instance.
    pub dispatcher: Arc<dyn Dispatcher>,
    /// Guest argv (`args[0]` is the program name); `None` for reactor
    /// deployments that do not model a CLI invocation.
    pub args: Option<Arc<Vec<String>>>,
    /// Mount registry preopened into the guest sandbox; `None` for
    /// deployments without `[[mount]]`s.
    pub mounts: Option<Arc<MountRegistry>>,
    /// Complete guest environment replacing host inheritance; `None` inherits
    /// the host env.
    pub env: Option<Arc<Vec<(String, String)>>>,
    /// Type-erased `omnia:plugins/loader` capability; `None` for hand-built
    /// store contexts, where every load refuses.
    pub loader: Option<Arc<dyn PluginLoader>>,
}

/// The fixed per-store state shared by every guest store context.
///
/// Construction policy (WASI inheritance, argv, the memory limit, and inert wRPC
/// view state) lives in [`StoreBase::new`] so it is documented and
/// unit-testable instead of being inlined in [`Runtime::store`](crate::Runtime::store).
pub struct StoreBase {
    /// The store's WASI resource table.
    pub table: ResourceTable,
    /// The store's WASI context (inherited env/stdin, host stdout/stderr).
    pub wasi: WasiCtx,
    /// The per-guest memory limiter the runtime installs on every [`Store`].
    ///
    /// [`Store`]: wasmtime::Store
    pub limits: StoreLimits,
    /// Per-store wRPC view state for host-mediated dynamic linking; inert
    /// unless the deployment declares `plugins` interfaces.
    pub wrpc: WrpcState,
    /// Type-erased host->guest dispatcher (e.g. `wasi-model`'s `resolve`); a
    /// fresh handle to the owning runtime. Inert unless a host binding reaches
    /// for it.
    pub dispatcher: Arc<dyn Dispatcher>,
    /// Mount registry: the startup-validated mounts also preopened into
    /// [`wasi`](Self::wasi). A consuming host crate reads it to match a lent
    /// `descriptor` back to its mount by directory identity. Empty unless the
    /// deployment configures `[[mount]]`s.
    pub mounts: Arc<MountRegistry>,
    /// Type-erased `omnia:plugins/loader` capability the `WasiPlugins` host
    /// binding reaches for. Absent in hand-built store contexts.
    pub loader: Option<Arc<dyn PluginLoader>>,
}

impl StoreBase {
    /// Build the fixed per-store state for a single guest invocation, applying
    /// the WASI construction policy shared by every deployment.
    ///
    /// Applies the guest environment (the explicit [`env`](StoreConfig::env)
    /// list when set, host inheritance otherwise), inherits stdin, wires
    /// stdout/stderr to the host streams, applies the configured argv, caps
    /// linear-memory growth, and creates fresh, inert wRPC view state.
    #[must_use]
    pub fn new(config: StoreConfig<'_>) -> Self {
        let mounts = config.mounts.unwrap_or_default();

        let mut wasi_builder = WasiCtxBuilder::new();
        match &config.env {
            Some(env) => {
                wasi_builder.envs(env);
            }
            None => {
                wasi_builder.inherit_env();
            }
        }
        wasi_builder.inherit_stdin().stdout(tokio::io::stdout()).stderr(tokio::io::stderr());
        if let Some(args) = &config.args {
            wasi_builder.args(args.as_slice());
        }

        // Preopen each authorized mount into the guest sandbox. The
        // registry was opened + validated once at startup, so a failure here is
        // rare (e.g. a mount removed mid-run); log and skip — the guest simply
        // can't lend that tree and the consuming host's identity match then
        // fails cleanly, with no ambient fallback.
        for entry in mounts.entries() {
            let perms = if entry.writable { FsPerms::ReadWrite } else { FsPerms::ReadOnly };
            if let Err(error) = wasi_builder.preopened_dir(&entry.host_path, &entry.name, perms) {
                tracing::warn!(
                    %error,
                    name = %entry.name,
                    path = %entry.host_path.display(),
                    "failed to preopen mount; guest will not see it",
                );
            }
        }

        Self {
            table: ResourceTable::new(),
            wasi: wasi_builder.build(),
            limits: StoreLimitsBuilder::new().memory_size(config.options.max_memory_bytes).build(),
            wrpc: WrpcState::new(),
            dispatcher: config.dispatcher,
            mounts,
            loader: config.loader,
        }
    }
}

/// The per-guest store context every deployment shares.
///
/// `StoreCtx<B>` pairs the fixed [`StoreBase`] with the deployment's connected
/// backend bundle `B` — the `runtime!`-generated `Backends`, or [`()`](unit) for
/// a backend-less deployment (such as a `mode: command` `wasi:cli` runtime). The
/// three fixed views (`WasiView`, `WrpcView`, `HasLimits`) are implemented below
/// against [`base`](Self::base); each host crate adds a blanket
/// `WasiXxxView for StoreCtx<B> where B: HasXxx`, so a deployment only supplies
/// the bundle and its `HasXxx` accessor impls (generated by the `runtime!`
/// macro).
///
/// This is the boilerplate the `runtime!` macro and hand-written runtimes
/// previously reproduced per deployment; hosting it here keeps it library code
/// reviewed once.
pub struct StoreCtx<B> {
    /// The fixed per-store state shared by every deployment.
    pub base: StoreBase,
    /// The deployment's connected backend bundle (cloned per store).
    pub backends: B,
}

impl<B: Send + 'static> WasiView for StoreCtx<B> {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.base.wasi,
            table: &mut self.base.table,
        }
    }
}

impl<B: Send + 'static> WrpcView for StoreCtx<B> {
    type Invoke = LinkClient;

    fn wrpc(&mut self) -> WrpcCtxView<'_, LinkClient> {
        self.base.wrpc.view(&mut self.base.table)
    }
}

impl<B: Send + 'static> HasLimits for StoreCtx<B> {
    fn limits(&mut self) -> &mut StoreLimits {
        &mut self.base.limits
    }
}

/// A backend bundle that can yield the `wasi:http` view for a [`StoreCtx`].
///
/// `wasi:http`'s view trait (`WasiHttpView`) is foreign — re-exported from
/// `wasmtime-wasi-http` — so its blanket impl on `StoreCtx<B>` can only live
/// here, where `StoreCtx` is local. Every other host owns its view trait and
/// blankets it in its own crate. The `runtime!` macro generates the bundle-side
/// impl of this trait directly.
pub trait HasHttp: Send {
    /// Borrow the `wasi:http` context as the linker-facing view, threading in
    /// the store's [`ResourceTable`].
    fn http_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a>;
}

impl<B: HasHttp + Send + 'static> WasiHttpView for StoreCtx<B> {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        self.backends.http_view(&mut self.base.table)
    }
}

/// Clone-on-read access to a store's startup-validated mount registry.
///
/// Lets a host crate match a lent `wasi:filesystem` descriptor against the
/// store's authorized mounts without carrying the registry on its own view.
pub trait HasMounts: Send {
    /// Clone a handle to the store's mount registry.
    fn mounts(&self) -> Arc<MountRegistry>;
}

impl<B: Send + 'static> HasMounts for StoreCtx<B> {
    fn mounts(&self) -> Arc<MountRegistry> {
        Arc::clone(&self.base.mounts)
    }
}

/// Clone-on-read access to a store's host->guest dispatcher.
///
/// Lets a host crate reach the dispatcher for host-mediated dynamic linking
/// without carrying it on its own view.
pub trait HasDispatcher: Send {
    /// Clone a handle to the store's host->guest dispatcher.
    fn dispatcher(&self) -> Arc<dyn Dispatcher>;
}

impl<B: Send + 'static> HasDispatcher for StoreCtx<B> {
    fn dispatcher(&self) -> Arc<dyn Dispatcher> {
        Arc::clone(&self.base.dispatcher)
    }
}
