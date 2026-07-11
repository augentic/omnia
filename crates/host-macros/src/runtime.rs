//! # Runtime macro configuration and expansion
//!
//! Parses `runtime!({ ... })` and expands it into a complete runtime module.

mod codegen;
mod parse;

use proc_macro2::TokenStream;
use quote::quote;

use crate::runtime::codegen::Codegen;
pub use crate::runtime::parse::{Config, Mode};

/// Generate the runtime module from a parsed [`Config`].
pub fn expand(config: &Config) -> TokenStream {
    let Codegen {
        mode,
        host_types,
        server_types,
        backends_ty,
        backends_def,
    } = Codegen::from(config);

    let mode = match mode {
        Mode::Server => quote!(omnia::Mode::Server),
        Mode::Command => quote!(omnia::Mode::Command),
    };

    quote! {
        mod runtime {
            use anyhow::Result;
            use omnia::futures::future;
            use omnia::Server;
            use omnia::tokio;
            use super::*;

            #backends_def

            struct Hooks;

            impl omnia::Wiring<#backends_ty> for Hooks {
                fn link(deployment: &mut omnia::Deployment<omnia::StoreCtx<#backends_ty>>) -> Result<()> {
                    #(deployment.host::<#host_types, #backends_ty>()?;)*
                    Ok(())
                }

                async fn serve(
                    runtime: &omnia::Runtime<#backends_ty>,
                ) -> Result<()> {
                    let servers: Vec<future::BoxFuture<'_, Result<()>>> = vec![
                        #(
                            Box::pin(#server_types.run(runtime)),
                        )*
                    ];
                    future::try_join_all(servers).await?;
                    Ok(())
                }
            }

            /// CLI entry point: parse the `run` grammar, then drive the
            /// deployment through this runtime's hosts and backends.
            #[tokio::main]
            pub async fn main() -> ::std::process::ExitCode {
                omnia::main::<#backends_ty, Hooks>(#mode).await
            }

            /// Drive one deployment through this runtime's hosts and backends,
            /// blocking until the guest completes.
            #[tokio::main]
            pub async fn drive(builder: omnia::DeploymentBuilder) -> Result<omnia::ExitStatus> {
                omnia::run::<#backends_ty, Hooks>(builder.mode(#mode)).await
            }
        }

        #[allow(unused_imports)]
        pub use runtime::{drive, main};
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::*;

    // Expand a `runtime!` config and pretty-print the output so snapshots are
    // readable and diffs are line-oriented.
    fn expand_pretty(input: proc_macro2::TokenStream) -> String {
        let config: Config = syn::parse2(input).expect("config parses");
        let file = syn::parse2::<syn::File>(expand(&config)).expect("expansion parses as a file");
        prettyplease::unparse(&file)
    }

    #[test]
    fn expand_server() {
        insta::assert_snapshot!(expand_pretty(quote!({
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
                WasiKeyValue: KeyValueDefault,
            },
        })));
    }

    #[test]
    fn expand_command() {
        insta::assert_snapshot!(expand_pretty(quote!({
            mode: command,
            hosts: {
                WasiOtel: OtelDefault,
            },
        })));
    }
}
