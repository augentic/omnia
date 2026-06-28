//! # One-shot `wasi:cli` command
//!
//! Command mode's counterpart to [`serve`](crate::serve): it `prepare`s the
//! runtime the same way, then drives the `wasi:cli/run` export of the sole
//! command-capable guest exactly once and returns its [`ExitStatus`]. Because a
//! command yields a value (unlike a long-lived trigger), the status is returned
//! directly rather than published out of band.

use anyhow::{Result, bail};
use wasmtime_wasi::p3::bindings::{Command, CommandPre};
use wasmtime_wasi::{I32Exit, WasiView};
use wrpc_wasmtime::WrpcView;

use crate::routing::TriggerRouter;
use crate::runtime::{ExitStatus, prepare};
use crate::traits::Runtime;

/// Drive the sole `wasi:cli/run` guest once and return its exit status.
///
/// Starts the runtime's background tasks and link serve side via `prepare`,
/// then probes for a command-capable guest with the same [`TriggerRouter`] the
/// other triggers use: a guest qualifies exactly when its `wasi:cli/run` export
/// resolves (`CommandPre::new` is `Ok`). The `cli` route table is empty today,
/// so a sole exporter is the catch-all, zero exporters is inert (clean
/// [`SUCCESS`](ExitStatus::SUCCESS)), and ">1 with no routes" is the same
/// ambiguity error the other triggers raise.
///
/// # Errors
///
/// Returns an error if `prepare` fails, if routing is ambiguous, if
/// instantiation fails, or if the guest traps. A guest `exit`/panic is *not* an
/// error: it surfaces through the returned [`ExitStatus`].
///
/// # Panics
///
/// Panics if the routed guest is absent from the registry — an invariant
/// [`TriggerRouter`] upholds (the id it returns came from the registry), so a
/// panic here signals a runtime bug.
pub async fn run<R>(runtime: &R) -> Result<ExitStatus>
where
    R: Runtime,
    R::StoreCtx: WasiView + WrpcView + 'static,
{
    // Identical startup to `serve`: background tasks + host-mediated links.
    prepare(runtime).await?;

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
        // >1 command-capable guest but no `[[route.cli]]` to disambiguate;
        // multi-command routing is deferred, so this is a clean error.
        bail!("multiple wasi:cli/run guests but no [[route.cli]] to disambiguate");
    };
    let guest = runtime.registry().get(guest_id).expect("a capable guest is registered");
    tracing::debug!(guest = %guest_id, "driving wasi:cli/run");

    // Instance-per-call, *through* `Runtime::instantiate`, so a command records
    // the same instantiation metrics every trigger does. argv is already in the
    // store via the `StoreBase` builder.
    let mut store = runtime.build_store(runtime.store());
    let instance = runtime.instantiate(guest.instance_pre(), &mut store).await?;
    let command = Command::new(&mut store, &instance)?;

    // Invoke once via the p3 concurrent convention (mirrors the HTTP host's
    // `run_concurrent`). `run_concurrent` wraps the call in its own `Result`,
    // and `call_run` returns `Result<Result<(), ()>>`: the outer layers carry
    // host traps (a guest `exit`/panic surfaces as `I32Exit`), the innermost is
    // the guest's `wasi:cli/run` result.
    let outcome =
        store.run_concurrent(async move |store| command.wasi_cli_run().call_run(store).await).await;

    let status = match outcome {
        Ok(Ok(Ok(()))) => ExitStatus::SUCCESS,
        Ok(Ok(Err(()))) => ExitStatus::from(1),
        Ok(Err(error)) | Err(error) => match error.downcast_ref::<I32Exit>() {
            Some(exit) => ExitStatus::from(exit.0),
            // A real host trap (not a guest exit) propagates to the caller.
            // `run_concurrent`/`call_run` yield `wasmtime::Error`, distinct from
            // `anyhow::Error` in wasmtime 46, so convert it.
            None => return Err(error.into()),
        },
    };

    tracing::info!(guest = %guest_id, code = status.code(), "wasi:cli/run exited");
    Ok(status)
}
