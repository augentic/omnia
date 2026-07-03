//! Per-store context. [`StoreBase`] holds the state identical for every
//! deployment (WASI table/context, memory limiter, wRPC view state, host→guest
//! dispatcher); [`StoreCtx`] pairs it with the deployment's backend bundle `B`
//! and implements the fixed `WasiView`/`WrpcView`/`HasLimits` views, while each
//! host crate blankets its own `WasiXxxView for StoreCtx<B> where B: HasXxx`.

use std::sync::Arc;

use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::p3::{WasiHttpCtxView, WasiHttpView};
use wrpc_wasmtime::{WrpcCtxView, WrpcView};

use crate::{Dispatcher, LinkClient, MountRegistry, RuntimeOptions, WrpcState};

/// Exposes a store context's [`StoreLimits`] so the runtime can install a
/// per-guest resource limiter on every [`Store`](wasmtime::Store) it creates.
pub trait HasLimits {
    /// Returns a mutable reference to the context's resource limits.
    fn limits(&mut self) -> &mut StoreLimits;
}

/// Type-state marker for a [`StoreBaseBuilder`] member that has been supplied,
/// carrying its value until [`build`](StoreBaseBuilder::build) consumes it.
pub struct Set<T>(T);

/// Type-state marker for a [`StoreBaseBuilder`] member that is not yet supplied.
pub struct Unset;

/// Type-state builder for [`StoreBase`], created by [`StoreBase::builder`].
///
/// The `O` and `D` type parameters track whether the required
/// [`options`](Self::options) and [`dispatcher`](Self::dispatcher) members have been
/// supplied: each starts as [`Unset`] and becomes `Set<…>` once its setter runs.
/// [`build`](Self::build) is implemented only when both are `Set`, so omitting
/// either is a compile error rather than a runtime panic. The optional
/// [`args`](Self::args) member defaults to empty and may be set in any state.
pub struct StoreBaseBuilder<O = Unset, D = Unset> {
    options: O,
    dispatcher: D,
    args: Option<Arc<Vec<String>>>,
    mounts: Option<Arc<MountRegistry>>,
}

impl<O, D> StoreBaseBuilder<O, D> {
    /// Set the guest argv (`args[0]` is the program name).
    ///
    /// Takes the runtime's shared argv handle so each store clones an `Arc`
    /// rather than the vector. Optional; defaults to empty for reactor
    /// deployments that do not model a CLI invocation.
    #[must_use]
    pub fn args(mut self, args: Arc<Vec<String>>) -> Self {
        self.args = Some(args);
        self
    }

    /// Set the mount registry preopened into the guest sandbox (RFC-55).
    ///
    /// Optional; defaults to an empty registry (no mounts) so reactor
    /// deployments without `[[mount]]`s — and the hand-written test runtimes —
    /// build unchanged. The `runtime!` macro threads the startup-validated
    /// registry here.
    #[must_use]
    pub fn mounts(mut self, mounts: Arc<MountRegistry>) -> Self {
        self.mounts = Some(mounts);
        self
    }
}

impl<D> StoreBaseBuilder<Unset, D> {
    /// Set the runtime options (required).
    ///
    /// Caps linear-memory growth at [`RuntimeOptions::max_memory_bytes`].
    #[must_use]
    pub fn options(self, options: &RuntimeOptions) -> StoreBaseBuilder<Set<&RuntimeOptions>, D> {
        StoreBaseBuilder {
            options: Set(options),
            dispatcher: self.dispatcher,
            args: self.args,
            mounts: self.mounts,
        }
    }
}

impl<O> StoreBaseBuilder<O, Unset> {
    /// Set the type-erased host->guest dispatcher (required).
    ///
    /// Pass a fresh handle to the owning [`Runtime`] so any host->guest call
    /// (such as `wasi-model`'s `resolve`) lands a new instance.
    ///
    /// [`Runtime`]: crate::Runtime
    #[must_use]
    pub fn dispatcher(
        self, dispatcher: Arc<dyn Dispatcher>,
    ) -> StoreBaseBuilder<O, Set<Arc<dyn Dispatcher>>> {
        StoreBaseBuilder {
            options: self.options,
            dispatcher: Set(dispatcher),
            args: self.args,
            mounts: self.mounts,
        }
    }
}

impl StoreBaseBuilder<Set<&RuntimeOptions>, Set<Arc<dyn Dispatcher>>> {
    /// Finish building the fixed per-store state, applying the WASI construction
    /// policy shared by every deployment.
    ///
    /// Inherits the host environment and stdin, wires stdout/stderr to the host
    /// streams, applies the configured argv, caps linear-memory growth, and
    /// creates fresh, inert wRPC view state.
    #[must_use]
    pub fn build(self) -> StoreBase {
        let Set(options) = self.options;
        let Set(dispatcher) = self.dispatcher;
        let mounts = self.mounts.unwrap_or_default();

        let mut wasi_builder = WasiCtxBuilder::new();
        wasi_builder
            .inherit_env()
            .inherit_stdin()
            .stdout(tokio::io::stdout())
            .stderr(tokio::io::stderr());
        if let Some(args) = &self.args {
            wasi_builder.args(args.as_slice());
        }

        // Preopen each authorized mount into the guest sandbox (RFC-55). The
        // registry was opened + validated once at startup, so a failure here is
        // rare (e.g. a mount removed mid-run); log and skip — the guest simply
        // can't lend that tree and the consuming host's identity match then
        // fails cleanly, with no ambient fallback.
        for entry in mounts.entries() {
            if let Err(error) = wasi_builder.preopened_dir(
                &entry.host_path,
                &entry.name,
                entry.dir_perms,
                entry.file_perms,
            ) {
                tracing::warn!(
                    %error,
                    name = %entry.name,
                    path = %entry.host_path.display(),
                    "failed to preopen mount; guest will not see it",
                );
            }
        }

        let wasi = wasi_builder.build();

        StoreBase {
            table: ResourceTable::new(),
            wasi,
            limits: StoreLimitsBuilder::new().memory_size(options.max_memory_bytes).build(),
            wrpc: WrpcState::new(),
            dispatcher,
            mounts,
        }
    }
}

/// The fixed per-store state shared by every guest store context.
///
/// Construction policy (WASI inheritance, argv, the memory limit, and inert wRPC
/// view state) lives in [`StoreBase::builder`] so it is documented and
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
    /// unless the deployment declares `link`s.
    pub wrpc: WrpcState,
    /// Type-erased host->guest dispatcher (e.g. `wasi-model`'s `resolve`); a
    /// fresh handle to the owning runtime. Inert unless a host binding reaches
    /// for it.
    pub dispatcher: Arc<dyn Dispatcher>,
    /// Mount registry (RFC-55): the startup-validated mounts also preopened into
    /// [`wasi`](Self::wasi). A consuming host crate reads it to match a lent
    /// `descriptor` back to its mount by directory identity. Empty unless the
    /// deployment configures `[[mount]]`s.
    pub mounts: Arc<MountRegistry>,
}

impl StoreBase {
    /// Begin building the fixed per-store state for a single guest invocation.
    ///
    /// [`options`](StoreBaseBuilder::options) and
    /// [`dispatcher`](StoreBaseBuilder::dispatcher) are required (the type-state
    /// builder will not expose [`build`](StoreBaseBuilder::build) until both are
    /// set); [`args`](StoreBaseBuilder::args) is optional.
    ///
    /// ```ignore
    /// let base = StoreBase::builder()
    ///     .options(self.options())
    ///     .dispatcher(Arc::new(self.clone()))
    ///     .build();
    /// ```
    #[must_use]
    pub const fn builder() -> StoreBaseBuilder {
        StoreBaseBuilder {
            options: Unset,
            dispatcher: Unset,
            args: None,
            mounts: None,
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

/// Clone-on-read access to a store's startup-validated mount registry (RFC-55).
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
