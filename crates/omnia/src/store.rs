//! # Fixed per-store state
//!
//! [`StoreBase`] is the slice of a guest store context that is identical for every
//! deployment: the WASI resource table and context, the per-guest memory
//! limiter, the wRPC view state backing host-mediated dynamic linking, and the
//! type-erased host->guest dispatcher. A concrete `StoreCtx` embeds one `StoreBase`
//! field plus its deployment-specific backend fields; the
//! [`StoreContext`](omnia_host_macros::StoreContext) derive implements the
//! three fixed views (`WasiView`, `WrpcView`, `HasLimits`) against it.

use std::sync::Arc;

use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder};

use crate::{HostDispatch, RuntimeOptions, WrpcState};

/// Type-state marker for a [`StoreBaseBuilder`] member that has been supplied,
/// carrying its value until [`build`](StoreBaseBuilder::build) consumes it.
pub struct Set<T>(T);

/// Type-state marker for a [`StoreBaseBuilder`] member that is not yet supplied.
pub struct Unset;

/// Type-state builder for [`StoreBase`], created by [`StoreBase::builder`].
///
/// The `O` and `D` type parameters track whether the required
/// [`options`](Self::options) and [`dispatch`](Self::dispatch) members have been
/// supplied: each starts as [`Unset`] and becomes `Set<…>` once its setter runs.
/// [`build`](Self::build) is implemented only when both are `Set`, so omitting
/// either is a compile error rather than a runtime panic. The optional
/// [`args`](Self::args) member defaults to empty and may be set in any state.
pub struct StoreBaseBuilder<O = Unset, D = Unset> {
    options: O,
    dispatch: D,
    args: Vec<String>,
}

impl<O, D> StoreBaseBuilder<O, D> {
    /// Set the guest argv (`args[0]` is the program name).
    ///
    /// Optional; defaults to empty for reactor deployments that do not model a
    /// CLI invocation.
    #[must_use]
    pub fn args(mut self, args: &[String]) -> Self {
        self.args = args.to_vec();
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
            dispatch: self.dispatch,
            args: self.args,
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
    pub fn dispatch(
        self, dispatch: Arc<dyn HostDispatch>,
    ) -> StoreBaseBuilder<O, Set<Arc<dyn HostDispatch>>> {
        StoreBaseBuilder {
            options: self.options,
            dispatch: Set(dispatch),
            args: self.args,
        }
    }
}

impl StoreBaseBuilder<Set<&RuntimeOptions>, Set<Arc<dyn HostDispatch>>> {
    /// Finish building the fixed per-store state, applying the WASI construction
    /// policy shared by every deployment.
    ///
    /// Inherits the host environment and stdin, wires stdout/stderr to the host
    /// streams, applies the configured argv, caps linear-memory growth, and
    /// creates fresh, inert wRPC view state.
    #[must_use]
    pub fn build(self) -> StoreBase {
        let Set(options) = self.options;
        let Set(host_dispatch) = self.dispatch;

        let wasi = WasiCtxBuilder::new()
            .inherit_env()
            .inherit_stdin()
            .stdout(tokio::io::stdout())
            .stderr(tokio::io::stderr())
            .args(&self.args)
            .build();

        StoreBase {
            table: ResourceTable::new(),
            wasi,
            limits: StoreLimitsBuilder::new().memory_size(options.max_memory_bytes).build(),
            wrpc: WrpcState::new(),
            host_dispatch,
        }
    }
}

/// The fixed per-store state shared by every guest store context.
///
/// Construction policy (WASI inheritance, argv, the memory limit, and inert wRPC
/// view state) lives in [`StoreBase::builder`] so it is documented and
/// unit-testable instead of being inlined in macro-generated `Runtime::store()`
/// output.
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
    pub host_dispatch: Arc<dyn HostDispatch>,
}

impl StoreBase {
    /// Begin building the fixed per-store state for a single guest invocation.
    ///
    /// [`options`](StoreBaseBuilder::options) and
    /// [`dispatch`](StoreBaseBuilder::dispatch) are required (the type-state
    /// builder will not expose [`build`](StoreBaseBuilder::build) until both are
    /// set); [`args`](StoreBaseBuilder::args) is optional.
    ///
    /// ```ignore
    /// let base = StoreBase::builder()
    ///     .options(self.options())
    ///     .dispatch(Arc::new(self.clone()))
    ///     .build();
    /// ```
    #[must_use]
    pub const fn builder() -> StoreBaseBuilder {
        StoreBaseBuilder {
            options: Unset,
            dispatch: Unset,
            args: Vec::new(),
        }
    }
}
