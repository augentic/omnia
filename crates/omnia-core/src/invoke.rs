//! The one primitive that drives a fresh callee instance, for host→guest and
//! guest→guest dispatch alike.

use std::fmt;
use std::time::Duration;

use anyhow::Context as _;
use tokio::task::JoinHandle;
use tracing::Instrument as _;
use wasmtime::component::{ComponentExportIndex, InstancePre, Val};

use crate::chain::{ChainCtx, Chained as _};
use crate::seam::StoreFactory;
use crate::value::handle_kind;

/// Everything needed to instantiate a callee and invoke one of its exports.
pub struct FreshCall<T: 'static> {
    /// Builds the callee's store.
    pub factory: StoreFactory<T>,
    /// The callee's pre-instantiated component.
    pub instance_pre: InstancePre<T>,
    /// The export to invoke, resolved once on the component rather than per call.
    pub export: ComponentExportIndex,
    /// Number of results the export returns.
    pub results: usize,
}

/// Why a fresh call did not return the callee's results.
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
            // Trap and Join are transparent — display shows the top of the
            // inner chain and `source` exposes the rest — so an `anyhow` wrap
            // renders the chain exactly once under `{:#}`.
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

struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Instantiate `call`'s component on a fresh store and invoke its export with
/// `args`, returning the results.
///
/// The callee runs at `ctx` (so nested hops count against the same chain) and,
/// when `bound` is given, must finish within it. The store is dropped when the
/// call completes (instance-per-call).
///
/// # Errors
///
/// Returns [`InvokeError::Timeout`] when `bound` elapses, [`InvokeError::Handle`]
/// when a result carries a store-bound handle, [`InvokeError::Trap`] when
/// instantiation, export resolution, or the call fails, and
/// [`InvokeError::Join`] when the callee task panics or is cancelled.
pub async fn call_fresh<T: Send + 'static>(
    call: FreshCall<T>, args: Vec<Val>, ctx: ChainCtx, bound: Option<Duration>,
) -> Result<Vec<Val>, InvokeError> {
    // Own task: wasmtime 48 forbids nested `call_async` on concurrency-enabled
    // stores, and an abandoned caller must abort the callee (`AbortOnDrop`).
    // `spawn` drops the caller's span; re-enter it so guest otel export parents
    // onto the host span.
    let callee = async move {
        let mut store = (call.factory)();
        let instance = call.instance_pre.instantiate_async(&mut store).await?;
        let func = instance
            .get_func(&mut store, call.export)
            .context("export resolved at serve time vanished")?;
        let mut out = vec![Val::Bool(false); call.results];
        func.call_async(&mut store, &args, &mut out).await?;
        Ok::<_, anyhow::Error>(out)
    }
    .in_chain(ctx)
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
