//! # Fixed per-store state
//!
//! [`StoreBase`] is the slice of a guest store context that is identical for every
//! deployment: the WASI resource table and context, the per-guest memory
//! limiter, the wRPC view state backing host-mediated dynamic linking, and the
//! type-erased host->guest dispatcher. A concrete `StoreCtx` embeds one `StoreBase`
//! field plus its deployment-specific backend fields; the
//! [`StoreContext`](omnia_runtime_macro::StoreContext) derive implements the
//! three fixed views (`WasiView`, `WrpcView`, `HasLimits`) against it.

use std::sync::Arc;

use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder};

use crate::{HostDispatch, RuntimeOptions, WrpcState};

/// The fixed per-store state shared by every guest store context.
///
/// Construction policy (WASI inheritance, the memory limit, and inert wRPC view
/// state) lives in [`StoreBase::new`] so it is documented and unit-testable instead
/// of being inlined in macro-generated `Runtime::store()` output.
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
    /// Build the fixed per-store state for a single guest invocation.
    ///
    /// Inherits the host environment and stdin, wires stdout/stderr to the host
    /// streams, caps linear-memory growth at
    /// [`RuntimeOptions::max_memory_bytes`], and creates fresh, inert wRPC view
    /// state. `host_dispatch` is a fresh handle to the owning [`Runtime`] so any
    /// host->guest call (such as `wasi-model`'s `resolve`) lands a new instance.
    ///
    /// [`Runtime`]: crate::Runtime
    #[must_use]
    pub fn new(options: &RuntimeOptions, host_dispatch: Arc<dyn HostDispatch>) -> Self {
        let wasi = WasiCtxBuilder::new()
            .inherit_env()
            .inherit_stdin()
            .stdout(tokio::io::stdout())
            .stderr(tokio::io::stderr())
            .build();

        Self {
            table: ResourceTable::new(),
            wasi,
            limits: StoreLimitsBuilder::new().memory_size(options.max_memory_bytes).build(),
            wrpc: WrpcState::new(),
            host_dispatch,
        }
    }
}
