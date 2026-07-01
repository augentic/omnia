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
        backends_ty,
        backends_def,
    } = Codegen::from(config);

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
