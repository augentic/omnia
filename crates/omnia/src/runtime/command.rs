//! One-shot `wasi:cli/run` command mode.

use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use wasmtime_wasi::I32Exit;
use wasmtime_wasi::p3::bindings::{Command, CommandPre};

use super::{ExitStatus, Runtime};
use crate::registry::{Guest, GuestId, TriggerRouter};
use crate::store::StoreCtx;

/// Run the command guest once, after the [`Runtime`] is assembled.
///
/// An explicit command guest (see
/// [`DeploymentBuilder::command_guest`](crate::DeploymentBuilder::command_guest))
/// goes through the ordinary [`ensure_guest`](Runtime::ensure_guest) lookup —
/// and hence resolve-on-miss — and fails the run if nothing supplies it.
/// Without one, the sole static `wasi:cli/run` exporter is the catch-all; a
/// deployment with no exporter is inert and exits `0`.
///
/// # Errors
///
/// Returns an error if the explicit command guest cannot be ensured, routing
/// is ambiguous, the guest cannot be instantiated, the run exceeds
/// `guest_timeout`, or the command traps without a guest exit code.
pub(super) async fn drive<B>(runtime: &Runtime<B>) -> Result<ExitStatus>
where
    B: Clone + Send + Sync + 'static,
{
    if let Some(id) = runtime.command_guest() {
        let id = id.clone();
        let guest = runtime
            .ensure_guest(&id, "wasi:cli/run")
            .await
            .with_context(|| format!("ensuring command guest `{id}`"))?;
        return run_guest(runtime, &id, &guest).await;
    }

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
    run_guest(runtime, guest_id, &guest).await
}

/// Instantiate `guest` and drive its `wasi:cli/run` once.
async fn run_guest<B>(
    runtime: &Runtime<B>, guest_id: &GuestId, guest: &Arc<Guest<StoreCtx<B>>>,
) -> Result<ExitStatus>
where
    B: Clone + Send + Sync + 'static,
{
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
