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
        host_types,
        host_impls,
        backend_idents,
        backend_types,
    } = Codegen::from(config);

    // the connected backend bundle threaded into `omnia::Runtime`.
    let (backends_ty, backends_def) = if backend_idents.is_empty() {
        (quote! { () }, quote! {})
    } else {
        (
            quote! { Backends },
            quote! {
                use omnia::Backend;

                #[derive(Clone)]
                struct Backends {#(
                    #backend_idents: #backend_types,
                )*}

                impl omnia::Backends for Backends {
                    async fn connect() -> Result<Self> {
                        let (#(#backend_idents,)*) = tokio::try_join!(
                            #(<#backend_types as Backend>::connect(),)*
                        )?;
                        Ok(Self { #(#backend_idents,)* })
                    }
                }

                #(#host_impls)*
            },
        )
    };

    quote! {
        mod runtime {
            use anyhow::Result;
            use omnia::tokio;
            use omnia::Server;

            use super::*;

            #backends_def

            struct Hooks;

            impl omnia::RuntimeHooks<#backends_ty> for Hooks {
                fn link(deployment: &mut omnia::Deployment<omnia::StoreCtx<#backends_ty>>) -> Result<()> {
                    #(deployment.host::<#host_types, #backends_ty>()?;)*
                    Ok(())
                }

                fn servers(
                    runtime: &omnia::Runtime<#backends_ty>,
                ) -> Vec<omnia::futures::future::BoxFuture<'_, Result<()>>> {
                    let mut servers: Vec<omnia::futures::future::BoxFuture<'_, Result<()>>> = vec![];
                    #(
                        if <#host_types as Server<#backends_ty>>::IS_SERVER {
                            servers.push(
                                Box::pin(#host_types.run(runtime))
                                    as omnia::futures::future::BoxFuture<'_, Result<()>>,
                            );
                        }
                    )*
                    servers
                }
            }

            #[tokio::main]
            pub async fn main() -> ::std::process::ExitCode {
                omnia::main::<#backends_ty, Hooks>(#command).await
            }
        }

        use runtime::main;
    }
}
