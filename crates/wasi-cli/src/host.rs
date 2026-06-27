//! # WASI CLI host
//!
//! See the crate docs. [`WasiCli`] is a [`Server`] that drives `wasi:cli/run`
//! once and records the guest's exit status in a shared cell.

use std::sync::{Arc, OnceLock};

use anyhow::{Result, bail};
use omnia::wasmtime_wasi::p3::bindings::{Command, CommandPre};
use omnia::wasmtime_wasi::{I32Exit, WasiView};
use omnia::{ExitStatus, Host, HostKind, Runtime, Server, TriggerRouter};
use wasmtime::component::Linker;

/// Host-side, one-shot trigger for `wasi:cli`.
///
/// Drives the `wasi:cli/run` export of the sole command-capable guest exactly
/// once and reports its [`ExitStatus`] through the shared cell handed to
/// [`WasiCli::new`].
#[derive(Debug, Clone)]
pub struct WasiCli {
    /// The one-shot's result, read by the generated `main` at the process
    /// boundary. The status rides this side channel because [`Server::run`] /
    /// [`omnia::serve`] return `Result<()>` and discard each server's value.
    exit: Arc<OnceLock<ExitStatus>>,
}

impl WasiCli {
    /// Create a `wasi:cli` trigger that publishes the guest's exit status to
    /// `exit`.
    #[must_use]
    pub const fn new(exit: Arc<OnceLock<ExitStatus>>) -> Self {
        Self { exit }
    }
}

impl<T> Host<T> for WasiCli {
    // `wasi:cli`'s imports are ambient (supplied by the base linker); a trigger
    // drives an *export*, so there is nothing to add here. Cf. the no-op
    // `Server` default in `omnia`.
    fn add_to_linker(_linker: &mut Linker<T>) -> Result<()> {
        Ok(())
    }
}

impl<R> Server<R> for WasiCli
where
    R: Runtime,
    R::StoreCtx: WasiView,
{
    const KIND: HostKind = HostKind::OneShot;

    async fn run(&self, state: &R) -> Result<()> {
        // Capability probe + routing — the same `TriggerRouter` HTTP and
        // messaging use. A guest is command-capable exactly when its
        // `wasi:cli/run` export resolves (`CommandPre::new` is `Ok`). The `cli`
        // route table is empty today, so a sole exporter is the catch-all, zero
        // is inert, and ">1 with no routes" is the same ambiguity error HTTP
        // raises.
        let routing = TriggerRouter::build(
            state.registry(),
            "cli",
            state.registry().routes().cli().clone(),
            |pre| CommandPre::new(pre.clone()).map(|_| ()),
        )?;
        if routing.is_inert() {
            tracing::info!("no guest exports wasi:cli/run; cli trigger inert");
            return Ok(());
        }
        let Some((guest_id, ())) = routing.catch_all() else {
            // >1 command-capable guest but no `[[route.cli]]` to disambiguate;
            // multi-command routing is deferred, so this is a clean error.
            bail!("multiple wasi:cli/run guests but no [[route.cli]] to disambiguate");
        };
        let guest = state.registry().get(guest_id).expect("a capable guest is registered");
        tracing::debug!(guest = %guest_id, "driving wasi:cli/run");

        // Instance-per-call, *through* `Runtime::instantiate`, so a command
        // records the same instantiation metrics every trigger does. argv is
        // already in the store via the `StoreBase` builder.
        let mut store = state.build_store(state.store());
        let instance = state.instantiate(guest.instance_pre(), &mut store).await?;
        let command = Command::new(&mut store, &instance)?;

        // Invoke once via the p3 concurrent convention (mirrors the HTTP host's
        // `run_concurrent`). `run_concurrent` wraps the call in its own
        // `Result`, and `call_run` returns `Result<Result<(), ()>>`: the outer
        // layers carry host traps (a guest `exit`/panic surfaces as `I32Exit`),
        // the innermost is the guest's `wasi:cli/run` result.
        let outcome = store
            .run_concurrent(async move |store| command.wasi_cli_run().call_run(store).await)
            .await;

        let status = match outcome {
            Ok(Ok(Ok(()))) => ExitStatus::SUCCESS,
            Ok(Ok(Err(()))) => ExitStatus::from(1),
            Ok(Err(error)) | Err(error) => match error.downcast_ref::<I32Exit>() {
                Some(exit) => ExitStatus::from(exit.0),
                // A real host trap (not a guest exit) propagates through `serve`.
                // `run_concurrent`/`call_run` yield `wasmtime::Error`, distinct
                // from `anyhow::Error` in wasmtime 46, so convert it.
                None => return Err(error.into()),
            },
        };

        tracing::info!(guest = %guest_id, code = status.code(), "wasi:cli/run exited");

        // Hand the status to the boundary and complete. `serve`'s
        // `try_join_all` resolves here (this is the only server in a command
        // deployment), and the generated `main` reads the cell.
        let _ = self.exit.set(status);
        Ok(())
    }
}
