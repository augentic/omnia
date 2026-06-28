//! # Runtime macro configuration and expansion
//!
//! Parses `runtime!({ ... })` and expands it into a complete runtime module.

mod codegen;
mod parse;

use proc_macro2::TokenStream;
use quote::quote;

use crate::runtime::codegen::Codegen;
pub use crate::runtime::parse::Config;

/// Generate the runtime module from a parsed [`Config`].
pub fn expand(config: &Config) -> TokenStream {
    let Codegen {
        command,
        command_assert,
        context_fields,
        backend_idents,
        store_ctx_fields,
        host_trait_impls,
        backend_types,
    } = Codegen::from(config);

    let connect_backends = if backend_idents.is_empty() {
        quote! {}
    } else {
        quote! {
            let (#(#backend_idents,)*) = tokio::try_join!(
                #(<#backend_types as Backend>::connect(),)*
            )?;
        }
    };

    quote! {
            mod runtime {
                use std::sync::Arc;

                use anyhow::Result;
                use omnia::tokio;
                use omnia::{
                    Backend, Compiled, Registry, Runtime, Server, StoreBase, StoreContext,
                    WorkingTreeRegistry,
                };

                use super::*;

                // Runtime state holding the guest registry and backend connections.
                #[derive(Clone, Runtime)]
                #[runtime(store = StoreCtx)]
                struct Context {
                    #[runtime(registry)]
                    registry: Arc<Registry<StoreCtx>>,
                    #[runtime(args)]
                    args: Arc<Vec<String>>,
                    #[runtime(preopens)]
                    working_trees: Arc<WorkingTreeRegistry>,
                    #(#context_fields,)*
                }

                impl Context {
                    // Creates a new runtime state by linking WASI interfaces and connecting to backends.
                    async fn new(mut compiled: Compiled<StoreCtx>) -> Result<Self> {
                        let args = Arc::new(compiled.args().to_vec());

                        #(compiled.host::<#host_trait_impls, Context>()?;)*
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
                #[derive(StoreContext)]
                pub struct StoreCtx {
                    #[base]
                    pub base: StoreBase,
                    #(#store_ctx_fields,)*
                }

                #command_assert

                #[tokio::main]
                pub async fn main() -> ::std::process::ExitCode {
                    omnia::main(#command, Context::new, |ctx| {
                        let mut servers = Vec::new();
                        #(
                            if <#host_trait_impls as Server<Context>>::IS_SERVER {
                                servers.push(Box::pin(
                                    <#host_trait_impls as Server<Context>>::run(&#host_trait_impls, ctx),
                                ));
                            }
                        )*
                        servers
                    }).await
                }
            }

            use runtime::main;
        }
}
