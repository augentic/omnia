//! # Generated Code Expansion
//!
//! Expands the generated code into a complete runtime implementation.

use proc_macro2::TokenStream;
use quote::quote;

use crate::generate::Generated;

pub fn expand(generated: Generated) -> TokenStream {
    let Generated {
        context_fields,
        store_ctx_fields,
        store_ctx_values,
        host_trait_impls,
        server_trait_impls,
        wasi_view_impls,
        main_fn,
    } = generated;

    quote! {
        mod runtime {
            use std::path::PathBuf;

            use anyhow::Result;
            use yetti::anyhow::Context as _;
            use yetti::futures::future::{BoxFuture, try_join_all};
            use yetti::tokio;
            use yetti::wasmtime::component::InstancePre;
            use yetti::wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
            use yetti::{Backend, Compiled, Server, State};

            use super::*;

            /// Run the specified wasm guest using the configured runtime.
            pub async fn run(wasm: PathBuf) -> Result<()> {
                let mut compiled = yetti::create(&wasm)
                    .with_context(|| format!("compiling {}", wasm.display()))?;
                let run_state = Context::new(&mut compiled)
                    .await
                    .context("preparing runtime state")?;
                run_state.start().await.context("starting runtime services")
            }

            /// Initiator state holding pre-instantiated components and backend
            /// connections.
            #[derive(Clone)]
            struct Context {
                instance_pre: InstancePre<StoreCtx>,
                #(pub #context_fields,)*
            }

            impl Context {
                /// Creates a new runtime state by linking WASI interfaces and
                /// connecting to backends.
                async fn new(compiled: &mut Compiled<StoreCtx>) -> Result<Self> {
                    // link enabled WASI components
                    #(compiled.link(#host_trait_impls)?;)*

                    Ok(Self {
                        instance_pre: compiled.pre_instantiate()?,
                        #(#context_fields::connect().await?,)*
                    })
                }

                /// Start servers
                /// N.B. for simplicity, all hosts are "servers" with a default
                /// implementation the does nothing
                async fn start(&self) -> Result<()> {
                    let futures: Vec<BoxFuture<'_, Result<()>>> = vec![
                        #(Box::pin(#server_trait_impls.run(self)),)*
                    ];
                    try_join_all(futures).await?;
                    Ok(())
                }
            }

            impl State for Context {
                type StoreCtx = StoreCtx;

                fn instance_pre(&self) -> &InstancePre<Self::StoreCtx> {
                    &self.instance_pre
                }

                fn store(&self) -> Self::StoreCtx {
                    let wasi_ctx = WasiCtxBuilder::new()
                        // .inherit_args()
                        .inherit_env()
                        .inherit_stdin()
                        .stdout(tokio::io::stdout())
                        .stderr(tokio::io::stderr())
                        .build();

                    StoreCtx {
                        table: ResourceTable::new(),
                        wasi: wasi_ctx,
                        #(#store_ctx_values,)*
                    }
                }
            }

            /// Per-guest instance data shared between the runtime and guest
            pub struct StoreCtx {
                pub table: ResourceTable,
                pub wasi: WasiCtx,
                #(pub #store_ctx_fields,)*
            }

            /// WASI View Implementations
            impl WasiView for StoreCtx {
                fn ctx(&mut self) -> WasiCtxView<'_> {
                    WasiCtxView {
                        ctx: &mut self.wasi,
                        table: &mut self.table,
                    }
                }
            }

            #(#wasi_view_impls)*
        }

        /// Main function
        #main_fn
    }
}
