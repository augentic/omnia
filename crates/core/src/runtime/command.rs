//! One-shot `wasi:cli/run` command mode.

use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use tracing::Instrument;
use wasmtime::AsContextMut as _;
use wasmtime_wasi::I32Exit;
use wasmtime_wasi::p3::bindings::{Command, CommandPre};

use super::{ExitStatus, Runtime};
use crate::host::Backends;
use crate::registry::{Guest, GuestId};

/// Run the command guest once, after the [`Runtime`] is assembled.
///
/// A guest marked `command = true` in the manifest resolves through
/// [`Runtime::guest`], so a declared guest loads here by being run. Without
/// a marked guest, the sole `wasi:cli/run` exporter among the guests loaded
/// at boot runs; a deployment with no exporter is inert and exits `0`.
///
/// # Errors
///
/// Returns an error if the explicit command guest is neither registered nor
/// declared or fails to load, two or more boot guests export `wasi:cli/run`
/// and none is marked, the guest cannot be instantiated, or the command traps
/// without a guest exit code.
pub async fn drive<B: Backends>(runtime: &Runtime<B>) -> Result<ExitStatus> {
    if let Some(id) = runtime.command_guest() {
        let id = id.clone();
        let guest = runtime
            .guest(&id)
            .await
            .with_context(|| format!("resolving the command guest `{id}`"))?;
        return run_guest(runtime, &id, &guest).await;
    }

    let commands: Vec<Arc<Guest<B>>> = runtime
        .registry()
        .guests()
        .filter(|guest| CommandPre::new(guest.instance_pre().clone()).is_ok())
        .collect();
    match commands.as_slice() {
        [] => {
            tracing::warn!("no guest exports wasi:cli/run; command mode is inert");
            Ok(ExitStatus::SUCCESS)
        }
        [guest] => run_guest(runtime, guest.id(), guest).await,
        many => {
            let names = many.iter().map(|guest| guest.id().as_str()).collect::<Vec<_>>();
            bail!(
                "{} guests export wasi:cli/run ({}); mark one `command = true` to choose the \
                 command guest",
                many.len(),
                names.join(", ")
            )
        }
    }
}

async fn run_guest<B: Backends>(
    runtime: &Runtime<B>, guest_id: &GuestId, guest: &Arc<Guest<B>>,
) -> Result<ExitStatus> {
    tracing::debug!(guest = %guest_id, "running wasi:cli/run");

    // the run competes with the triggers serving beside it, so it waits for a
    // slot of the gate
    let slot = runtime.slot().await;

    // a command chain root: its link hops run uncapped, like the run itself
    let (mut entered, instance) = runtime.enter_command(guest, slot).await?;
    let command = Command::new(&mut entered, &instance)?;

    // the host span a guest's root spans join through `omnia:otel`
    let outcome = entered
        .as_context_mut()
        .run_concurrent(async move |store| command.wasi_cli_run().call_run(store).await)
        .instrument(tracing::info_span!("cli-run"))
        .await;

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
