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
        server_types,
        backends_ty,
        backends_def,
    } = Codegen::from(config);

    quote! {
        mod runtime {
            use anyhow::Result;
            use omnia::futures::future;
            use omnia::Server;
            use omnia::tokio;
            use super::*;

            #backends_def

            struct Hooks;

            impl omnia::RuntimeHooks<#backends_ty> for Hooks {
                fn link(deployment: &mut omnia::Deployment<omnia::StoreCtx<#backends_ty>>) -> Result<()> {
                    #(deployment.host::<#host_types, #backends_ty>()?;)*
                    Ok(())
                }

                fn serve(
                    runtime: &omnia::Runtime<#backends_ty>,
                ) -> impl ::std::future::Future<Output = Result<()>> + Send {
                    async {
                        let servers: Vec<future::BoxFuture<'_, Result<()>>> = vec![
                            #(
                                Box::pin(#server_types.run(runtime)),
                            )*
                        ];
                        future::try_join_all(servers).await?;
                        Ok(())
                    }
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
