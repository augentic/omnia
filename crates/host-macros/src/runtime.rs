//! # Runtime macro configuration and expansion
//!
//! Parses `runtime!({ ... })` and expands it into a complete runtime module.

mod codegen;
mod parse;

use proc_macro2::TokenStream;
use quote::quote;

use crate::runtime::codegen::{Codegen, Wiring};
pub use crate::runtime::parse::Config;

/// Generate the runtime module from a parsed [`Config`].
pub fn expand(config: &Config) -> TokenStream {
    let Codegen {
        mode,
        wiring,
        main_options,
        manifest,
    } = Codegen::from(config);
    let Wiring {
        backends_ty,
        backends_def,
        bounds,
        link,
        servers,
    } = wiring;

    let mode = mode.tokens();
    let manifest = manifest
        .unwrap_or_else(|| quote! { omnia::ManifestSource::Inline(omnia::Manifest::new()) });

    quote! {
        mod runtime {
            // every path resolves through the facade, the embedder's one dependency
            use omnia::anyhow::Result;
            use omnia::futures::future;
            use omnia::tokio;
            use super::*;

            #backends_def

            /// This runtime's host wiring, generic over the bundle carrying
            /// its hosts' contexts: the compiled-in bundle under `main` and
            /// `run`, any other under `run_with`.
            pub struct Hooks;

            impl<B> omnia::Wiring<B> for Hooks
            where
                B: omnia::Backends,
                #(#bounds,)*
            {
                fn link(deployment: &mut omnia::Deployment<B>) -> Result<()> {
                    #link
                    Ok(())
                }

                async fn serve(runtime: &omnia::Runtime<B>) -> Result<()> {
                    let servers: Vec<future::BoxFuture<'_, Result<()>>> = #servers;
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
                let deployment = builder.mode(#mode).build::<#backends_ty>().await?;
                omnia::run::<#backends_ty, Hooks>(deployment).await
            }

            /// Run one deployment through this runtime's hosts over a bundle
            /// already in hand — nothing connects.
            pub async fn run_with<B>(
                builder: omnia::DeploymentBuilder, backends: B,
            ) -> Result<omnia::ExitStatus>
            where
                B: omnia::Backends,
                Hooks: omnia::Wiring<B>,
            {
                let deployment = builder.mode(#mode).build::<B>().await?;
                omnia::run_with::<B, Hooks>(deployment, backends).await
            }
        }

        #[allow(unused_imports, reason = "a nested-module invocation uses only some re-exports")]
        pub use runtime::{Hooks, main, manifest, run, run_with};
    }
}
