//! One-shot `wasi:cli/run` command mode.

use anyhow::{Result, bail};
use wasmtime_wasi::I32Exit;
use wasmtime_wasi::p3::bindings::{Command, CommandPre};

use super::{ExitStatus, Runtime};
use crate::registry::TriggerRouter;

/// Run the routed `wasi:cli/run` guest once. Caller must have called [`prepare`](crate::runtime::prepare).
///
/// # Errors
///
/// Returns an error if routing is ambiguous, the guest cannot be instantiated,
/// or the command traps without a guest exit code.
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
    let guest = runtime.registry().get(guest_id).expect("a capable guest is registered");
    tracing::debug!(guest = %guest_id, "driving wasi:cli/run");

    let mut store = runtime.build_store(runtime.store());
    let instance = runtime.instantiate(guest.instance_pre(), &mut store).await?;
    let command = Command::new(&mut store, &instance)?;

    let outcome =
        store.run_concurrent(async move |store| command.wasi_cli_run().call_run(store).await).await;

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
