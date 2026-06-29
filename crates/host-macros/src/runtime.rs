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
        bundle_fields,
        accessor_impls,
        backend_idents,
        backend_types,
        host_trait_impls,
    } = Codegen::from(config);

    // The connected backend bundle threaded into `omnia::Runtime`.
    let (bundle_ty, bundle_def) = if backend_idents.is_empty() {
        (quote! { () }, quote! {})
    } else {
        (
            quote! { Backends },
            quote! {
                use omnia::Backend;

                // One connected backend per declared `Host: Backend` wiring.
                #[derive(Clone)]
                struct Backends {
                    #(#bundle_fields,)*
                }

                impl omnia::Backends for Backends {
                    async fn connect() -> Result<Self> {
                        let (#(#backend_idents,)*) = tokio::try_join!(
                            #(<#backend_types as Backend>::connect(),)*
                        )?;
                        Ok(Self { #(#backend_idents,)* })
                    }
                }

                #(#accessor_impls)*
            },
        )
    };

    // A `command: true` deployment must not link a long-lived trigger server;
    // the host list's `IS_SERVER` flags surface that as a compile error.
    let command_assert = if command {
        quote! {
            const _: () = omnia::assert_hosts(&[
                #( <#host_trait_impls as Server<#bundle_ty>>::IS_SERVER, )*
            ]);
        }
    } else {
        quote! {}
    };

    quote! {
        mod runtime {
            use anyhow::Result;
            use omnia::tokio;
            use omnia::Server;

            use super::*;

            #bundle_def
            #command_assert

            #[tokio::main]
            pub async fn main() -> ::std::process::ExitCode {
                omnia::main::<#bundle_ty, _, _>(
                    #command,
                    |deployment| {
                        #(deployment.host::<#host_trait_impls, #bundle_ty>()?;)*
                        Ok(())
                    },
                    |runtime| {
                        let mut servers: Vec<omnia::futures::future::BoxFuture<'_, Result<()>>> = vec![];
                        #(
                            if <#host_trait_impls as Server<#bundle_ty>>::IS_SERVER {
                                servers.push(
                                    Box::pin(#host_trait_impls.run(runtime))
                                        as omnia::futures::future::BoxFuture<'_, Result<()>>,
                                );
                            }
                        )*
                        servers
                    },
                ).await
            }
        }

        use runtime::main;
    }
}
