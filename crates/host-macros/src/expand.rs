//! # Runtime macro expansion
//!
//! Expands the parsed runtime configuration into a complete runtime implementation.

use proc_macro2::TokenStream;
use quote::quote;

use crate::runtime::{Config, Expanded};

/// Generate the runtime module from a parsed [`Config`].
pub fn expand(config: &Config) -> TokenStream {
    let Expanded {
        context_fields,
        backend_idents,
        store_ctx_fields,
        host_trait_impls,
    } = config.expanded();
    let command = config.command;
    let command_guard = config.command_guard();
    let connect_backends = config.connect_backends();
    let servers = config.servers();

    quote! {
        mod runtime {
            use std::path::PathBuf;
            use std::sync::Arc;

            use anyhow::Result;
            use omnia::anyhow::Context as _;
            use omnia::tokio;
            use omnia::{
                Backend, Compiled, Registry, Runtime, Server, StoreBase, StoreContext,
                WorkingTreeRegistry,
            };

            use super::*;

            #command_guard

            // Runtime state holding the guest registry and backend connections.
            #[derive(Clone, Runtime)]
            #[runtime(store = StoreCtx)]
            struct Context {
                #[runtime(registry)]
                registry: Arc<Registry<StoreCtx>>,
                // Guest argv threaded into every store (empty for servers; in
                // command mode `args[0]` is the program name).
                #[runtime(args)]
                args: Arc<Vec<String>>,
                // Working-tree contains startup-validated preopens when the deployment
                // configures `[[mount]]`s or sets `OMNIA_WORKING_TREE`.
                #[runtime(preopens)]
                working_trees: Arc<WorkingTreeRegistry>,
                #(#context_fields,)*
            }

            impl Context {
                /// Creates a new runtime state by linking WASI interfaces and connecting to backends.
                async fn new(mut compiled: Compiled<StoreCtx>) -> Result<Self> {
                    let args = Arc::new(compiled.args().to_vec());

                    // link enabled WASI components
                    #(compiled.host::<#host_trait_impls>()?;)*

                    // connect to all backends concurrently
                    #connect_backends

                    // snapshot the startup-validated working-tree
                    let working_trees = compiled.working_trees();

                    // build the store context
                    Ok(Self {
                        registry: Arc::new(compiled.build()?),
                        args,
                        working_trees,
                        #(#backend_idents,)*
                    })
                }
            }

            /// Per-guest instance data shared between the runtime and the guest.
            ///
            /// The `StoreContext` derive implements `WasiView`, `WrpcView`, and
            /// `HasLimits` against `base`, plus one host view per `#[wasi(...)]`
            /// backend field.
            #[derive(StoreContext)]
            pub struct StoreCtx {
                #[base]
                pub base: StoreBase,
                #(#store_ctx_fields,)*
            }

            /// Build runtime state from the parsed CLI inputs and drive the
            /// deployment to the guest's exit status (or a host error).
            async fn run(
                wasm: Option<PathBuf>, config: Option<PathBuf>, args: Vec<String>,
            ) -> Result<omnia::ExitStatus> {
                let compiled = omnia::RegistryBuilder::new()
                    .wasm(wasm)
                    .config(config)
                    .args(args)
                    .command(#command)
                    .compile::<StoreCtx>()
                    .await
                    .context("building runtime")?;
                let run_state = Context::new(compiled)
                    .await
                    .context("preparing runtime state")?;

                omnia::drive(&run_state, #command, #servers)
                    .await
                    .context("running deployment")
            }

            /// Parse the CLI and drive the deployment to a process exit code: the
            /// guest's status for a one-shot `command`, success for a long-lived
            /// server's clean shutdown, or failure on a host error.
            #[tokio::main]
            pub async fn main() -> ::std::process::ExitCode {
                omnia::run_main(run).await
            }
        }

        use runtime::main;
    }
}
