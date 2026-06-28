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
        store_ctx_fields,
        bundle_fields,
        store_assignments,
        backend_idents,
        backend_types,
        host_trait_impls,
    } = Codegen::from(config);

    // The connected backend bundle threaded into `omnia::Context`. A deployment
    // with no backends rides the `()` bundle (`omnia::Backends for ()`), so no
    // bundle type or `connect` impl is generated.
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
            },
        )
    };

    quote! {
        mod runtime {
            use anyhow::Result;
            use omnia::tokio;
            use omnia::{Server, StoreBase, StoreContext};

            use super::*;

            /// Per-guest instance data shared between the runtime and the guest.
            #[derive(StoreContext)]
            pub struct StoreCtx {
                #[base]
                pub base: StoreBase,
                #(#store_ctx_fields,)*
            }

            #bundle_def

            // Clone each connected backend into the host-view field it backs.
            impl omnia::BuildStore<#bundle_ty> for StoreCtx {
                fn build_store(base: StoreBase, backends: &#bundle_ty) -> Self {
                    Self {
                        base,
                        #(#store_assignments,)*
                    }
                }
            }

            // The deployment's concrete host runtime.
            type Ctx = omnia::Context<StoreCtx, #bundle_ty>;

            #command_assert

            #[tokio::main]
            pub async fn main() -> ::std::process::ExitCode {
                omnia::main::<Ctx, _, _, _>(
                    #command,
                    |compiled| Ctx::new(compiled, |c| {
                        #(c.host::<#host_trait_impls, Ctx>()?;)*
                        Ok(())
                    }),
                    |ctx| {
                        let mut servers: Vec<omnia::futures::future::BoxFuture<'_, Result<()>>> = vec![];
                        #(
                            if <#host_trait_impls as Server<Ctx>>::IS_SERVER {
                                servers.push(
                                    Box::pin(#host_trait_impls.run(ctx))
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
