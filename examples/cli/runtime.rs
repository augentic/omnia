//! CLI command example runtime.
//!
//! A hand-written host that loads a single `wasi:cli/command` guest through the
//! `omnia` registry pipeline, injects argv into its WASI context, and invokes
//! the guest's `wasi:cli/run` export exactly once — exiting with the guest's
//! status. Modelled on `crates/omnia/tests/linking.rs`; see `README.md`.
//!
//! Two things the floor does not yet provide for a command are handled locally:
//! nothing else invokes `wasi:cli/run`, and `StoreBase` never sets guest argv.
//! The `run` func lives inside the versioned `wasi:cli/run@…` instance rather
//! than at the component root, so the typed `CommandPre` bindings (not
//! `Instance::get_func`) perform the export lookup.

cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use std::path::PathBuf;
        use std::sync::Arc;

        use anyhow::{Context, Result};
        use omnia::wasmtime_wasi::p2::bindings::CommandPre;
        use omnia::wasmtime_wasi::{I32Exit, WasiCtxBuilder};
        use omnia::{Registry, RegistryBuilder, Runtime, StoreBase, StoreContext};

        /// Per-store context mirroring a macro-generated `StoreCtx`: the fixed
        /// `StoreBase` state, with the `WasiView` / `WrpcView` / `HasLimits`
        /// impls supplied by the `StoreContext` derive. A command needs no host
        /// backend.
        #[derive(StoreContext)]
        struct CliCtx {
            #[base]
            base: StoreBase,
        }

        /// A minimal `Runtime` over a one-guest registry that injects the
        /// guest's argv when building each store.
        #[derive(Clone)]
        struct CliRuntime {
            registry: Arc<Registry<CliCtx>>,
            /// Guest argv; `args[0]` is the program name.
            args: Arc<Vec<String>>,
        }

        impl Runtime for CliRuntime {
            type StoreCtx = CliCtx;

            fn store(&self) -> CliCtx {
                // `StoreBase::new` wires env + stdio but omits argv, so rebuild
                // `wasi` with `.args(...)`. `base.wasi` is a public field, so the
                // override stays local to the example — no floor change.
                let mut base = StoreBase::new(self.options(), Arc::new(self.clone()));
                base.wasi = WasiCtxBuilder::new()
                    .inherit_env()
                    .inherit_stdin()
                    .stdout(tokio::io::stdout())
                    .stderr(tokio::io::stderr())
                    .args(&self.args[..])
                    .build();
                CliCtx { base }
            }

            fn registry(&self) -> &Registry<Self::StoreCtx> {
                &self.registry
            }
        }

        #[tokio::main]
        async fn main() -> Result<()> {
            // The example owns its argv (it is not the `omnia` CLI):
            // `cargo run --example cli -- <wasm> greet Ada`. Take the first
            // argument as the component path and forward the rest as the guest's
            // argv, with a program name at index 0.
            let mut argv = std::env::args().skip(1);
            let wasm = argv.next().context("usage: cli <wasm> [guest args...]")?;
            let mut guest_args = vec!["cli".to_string()];
            guest_args.extend(argv);

            let registry = RegistryBuilder::new()
                .wasm(PathBuf::from(wasm))
                .compile::<CliCtx>()
                .await?
                .build()?;
            let runtime = CliRuntime {
                registry: Arc::new(registry),
                args: Arc::new(guest_args),
            };

            // Single-file shorthand => exactly one guest. `CommandPre::new`
            // re-uses the registry's `InstancePre` and front-loads the
            // `wasi:cli/run` export lookup (it errors for a non-command guest —
            // the capability probe a real trigger would use to pick a target).
            let guest = runtime.registry().guests().next().context("a guest is registered")?;
            let mut store = runtime.build_store(runtime.store());
            let pre = CommandPre::new(guest.instance_pre().clone())?;
            let command = pre.instantiate_async(&mut store).await?;
            // `call_run`'s outer `Result` is a host-side trap; the inner is the
            // guest's `wasi:cli/run` result. A guest `std::process::exit` (or a
            // panic) surfaces as an `I32Exit` error rather than `Ok(Err(()))`, so
            // honor its status code instead of reporting it as a host failure —
            // the same shape the `wasmtime` CLI's command runner uses.
            match command.wasi_cli_run().call_run(&mut store).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(())) => std::process::exit(1),
                Err(error) => {
                    if let Some(exit) = error.downcast_ref::<I32Exit>() {
                        std::process::exit(exit.0);
                    }
                    Err(error.into())
                }
            }
        }
    } else {
        fn main() {}
    }
}
