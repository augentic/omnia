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
        mode,
        host_types,
        backends_ty,
        backends_def,
        main_options,
        manifest,
    } = Codegen::from(config);

    let mode = mode.tokens();
    let manifest = manifest
        .unwrap_or_else(|| quote! { omnia::ManifestSource::Inline(omnia::Manifest::new()) });

    quote! {
        mod runtime {
            // every path resolves through the facade, the embedder's one dependency
            use omnia::anyhow::Result;
            use omnia::futures::future;
            use omnia::Server;
            use omnia::tokio;
            use super::*;

            #backends_def

            /// This runtime's host wiring, generic over the bundle carrying
            /// its hosts' contexts: the compiled-in bundle under `main` and
            /// `run`, any other under `run_with`.
            pub struct Hooks;

            impl<B> omnia::Wiring<B> for Hooks
            where
                B: Clone + Send + Sync + 'static,
                #(B: omnia::Provides<#host_types>,)*
            {
                fn link(deployment: &mut omnia::Deployment<omnia::StoreCtx<B>>) -> Result<()> {
                    #(deployment.host::<#host_types, B>()?;)*
                    Ok(())
                }

                async fn serve(runtime: &omnia::Runtime<B>) -> Result<()> {
                    // capability hosts resolve at once through `Server`'s no-op default
                    let servers: Vec<future::BoxFuture<'_, Result<()>>> = vec![
                        #(
                            Box::pin(#host_types.run(runtime)),
                        )*
                    ];
                    future::try_join_all(servers).await?;
                    Ok(())
                }
            }

            /// The deployment compiled in here (`manifest:` or the inline
            /// manifest keys; empty when neither is declared), for an
            /// embedder to overlay before `run_with`.
            pub fn manifest() -> omnia::ManifestSource {
                #manifest
            }

            /// Entry point: run the compiled-in deployment through this
            /// runtime's hosts and backends (raw argv passthrough for a
            /// command deployment compiled in here, otherwise the standard
            /// `run` grammar).
            #[tokio::main]
            pub async fn main() -> ::std::process::ExitCode {
                omnia::main::<#backends_ty, Hooks>(#main_options).await
            }

            /// Run one deployment through this runtime's hosts and backends,
            /// blocking until the guest completes.
            #[tokio::main]
            pub async fn run(builder: omnia::DeploymentBuilder) -> Result<omnia::ExitStatus> {
                let deployment = builder.mode(#mode).build::<omnia::StoreCtx<#backends_ty>>().await?;
                omnia::run::<#backends_ty, Hooks>(deployment).await
            }

            /// Run one deployment through this runtime's hosts over a bundle
            /// already in hand — nothing connects.
            pub async fn run_with<B>(
                builder: omnia::DeploymentBuilder, backends: B,
            ) -> Result<omnia::ExitStatus>
            where
                B: Clone + Send + Sync + 'static,
                Hooks: omnia::Wiring<B>,
            {
                let deployment = builder.mode(#mode).build::<omnia::StoreCtx<B>>().await?;
                omnia::run_with::<B, Hooks>(deployment, backends).await
            }
        }

        #[allow(unused_imports, reason = "a nested-module invocation uses only some re-exports")]
        pub use runtime::{Hooks, main, manifest, run, run_with};
    }
}
