//! The primitive that drives a fresh callee instance for every dispatched call.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use tokio::task::JoinHandle;
use tracing::Instrument as _;
use wasmtime::component::{ComponentExportIndex, InstancePre, Val};
use wasmtime::{PoolConcurrencyLimitError, Store};

use crate::chain::ChainCtx;
use crate::runtime::Slot;
use crate::store::StoreCtx;
use crate::value::handle_kind;

// builds a fresh, fully configured store for one guest at the chain context
// its call runs at
pub type StoreFactory<B> = Arc<dyn Fn(ChainCtx) -> Store<StoreCtx<B>> + Send + Sync>;

pub struct FreshCall<B: 'static> {
    pub factory: StoreFactory<B>,
    pub instance_pre: InstancePre<StoreCtx<B>>,
    pub export: ComponentExportIndex,
    pub results: usize,
}

/// Why a dispatched call did not return its callee's results.
///
/// An error from [`Dispatcher::invoke`](crate::Dispatcher::invoke) downcasts
/// to it when the call reached its callee.
#[derive(Debug)]
pub enum InvokeError {
    /// The wall-clock bound elapsed; the callee task was aborted.
    Timeout(Duration),
    /// A result carried a store-bound handle of the named kind.
    Handle(&'static str),
    /// Instantiation, export resolution, or the call itself failed.
    Trap(anyhow::Error),
    /// The callee task panicked or was cancelled.
    Join(tokio::task::JoinError),
}

impl fmt::Display for InvokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout(bound) => write!(f, "timed out after {bound:?}"),
            Self::Handle(kind) => write!(f, "a {kind} handle cannot cross the link seam"),
            // transparent, so an `anyhow` wrap renders the chain once under `{:#}`
            Self::Trap(err) => write!(f, "{err}"),
            Self::Join(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for InvokeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Trap(err) => err.chain().nth(1),
            Self::Join(err) => std::error::Error::source(err),
            Self::Timeout(_) | Self::Handle(_) => None,
        }
    }
}

// The callee's store carries `ctx`, so nested hops count against the same
// chain, and drops when the call ends (instance-per-call). A root call's
// `slot` admits it for as long as that store lives, however the callee task
// ends.
pub async fn call_fresh<B: Send + 'static>(
    call: FreshCall<B>, slot: Option<Slot>, args: Vec<Val>, ctx: ChainCtx, bound: Option<Duration>,
) -> Result<Vec<Val>, InvokeError> {
    // own task, so the caller cannot starve the callee and an abandoned caller
    // aborts it; the bound wraps the whole call, as wasmtime's `run_concurrent`
    // docs recommend; the span `spawn` drops is re-entered for otel parenting
    let callee = async move {
        let mut callee = Callee {
            store: (call.factory)(ctx),
            _slot: slot,
        };
        let instance = call
            .instance_pre
            .instantiate_async(&mut callee.store)
            .await
            .map_err(explain_full_pool)?;
        let func = instance
            .get_func(&mut callee.store, call.export)
            .context("export resolved before instantiation vanished")?;
        let mut out = vec![Val::Bool(false); call.results];
        func.call_async(&mut callee.store, &args, &mut out).await.map_err(explain_full_pool)?;
        Ok::<_, anyhow::Error>(out)
    }
    .in_current_span();

    let mut callee = AbortOnDrop(tokio::spawn(callee));

    let out = match bound {
        Some(limit) => tokio::time::timeout(limit, &mut callee.0)
            .await
            .map_err(|_elapsed| InvokeError::Timeout(limit))?,
        None => (&mut callee.0).await,
    }
    .map_err(InvokeError::Join)?
    .map_err(InvokeError::Trap)?;

    if let Some(kind) = out.iter().find_map(handle_kind) {
        return Err(InvokeError::Handle(kind));
    }

    Ok(out)
}

struct Callee<B: 'static> {
    // fields drop in declaration order, so the pool has the store's instance
    // back before the slot admits the next invocation
    store: Store<StoreCtx<B>>,
    _slot: Option<Slot>,
}

// A full pool is a sizing fault, so its error names the two variables that
// size the pool against the gate.
pub fn explain_full_pool(error: wasmtime::Error) -> anyhow::Error {
    let full = error.is::<PoolConcurrencyLimitError>();
    let error = anyhow::Error::from(error);
    if full {
        error.context(
            "the instance pool is full; set MAX_CONCURRENT_INVOCATIONS below POOL_MAX_INSTANCES \
             to leave room for calls between guests, or raise POOL_MAX_INSTANCES",
        )
    } else {
        error
    }
}

struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
