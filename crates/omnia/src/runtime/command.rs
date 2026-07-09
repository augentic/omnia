//! One-shot `wasi:cli/run` command mode.

use anyhow::{Context as _, Result, bail};
use wasmtime_wasi::I32Exit;
use wasmtime_wasi::p3::bindings::{Command, CommandPre};

use super::{ExitStatus, Runtime};
use crate::registry::TriggerRouter;

/// Run the routed `wasi:cli/run` guest once, after the [`Runtime`] is assembled.
///
/// # Errors
///
/// Returns an error if routing is ambiguous, the guest cannot be instantiated,
/// the run exceeds `guest_timeout`, or the command traps without a guest exit
/// code.
pub(super) async fn drive<B>(runtime: &Runtime<B>) -> Result<ExitStatus>
where
    B: Clone + Send + Sync + 'static,
{
    let routing = TriggerRouter::build(
        runtime.registry(),
        "cli",
        runtime.registry().routes().cli().clone(),
        |pre| CommandPre::new(pre.clone()).map(|_| ()),
    )?;
    if routing.is_inert() {
        tracing::info!("no guest exports wasi:cli/run; cli trigger inert");
        return Ok(ExitStatus::SUCCESS);
    }
    let Some((guest_id, ())) = routing.catch_all() else {
        bail!("multiple wasi:cli/run guests but no [[route.cli]] to disambiguate");
    };
    let guest = runtime
        .registry()
        .get(guest_id)
        .with_context(|| format!("routed guest `{guest_id}` is not registered"))?;
    tracing::info!(guest = %guest_id, "running wasi:cli/run");

    let mut store = runtime.build_store(runtime.store());
    let instance = runtime.instantiate(guest.instance_pre(), &mut store).await?;
    let command = Command::new(&mut store, &instance)?;

    // The same wall-clock bound every other trigger applies to guest work;
    // long-running commands raise GUEST_TIMEOUT_MS.
    let timeout = runtime.options().guest_timeout;
    let run = store.run_concurrent(async move |store| command.wasi_cli_run().call_run(store).await);
    let outcome = tokio::time::timeout(timeout, run)
        .await
        .map_err(|_elapsed| anyhow::anyhow!("wasi:cli/run timed out after {timeout:?}"))?;

    let status = match outcome {
        Ok(Ok(Ok(()))) => ExitStatus::SUCCESS,
        Ok(Ok(Err(()))) => ExitStatus::from(1),
        Ok(Err(error)) | Err(error) => match error.downcast_ref::<I32Exit>() {
            Some(exit) => ExitStatus::from(exit.0),
            None => return Err(error.into()),
        },
    };

    tracing::info!(guest = %guest_id, code = status.code(), "wasi:cli/run exited");
    Ok(status)
}
