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
        main_options,
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

            /// Entry point: run the compiled-in deployment through this
            /// runtime's hosts and backends (the standard `run` grammar, or
            /// raw argv passthrough under the `program:` key).
            #[tokio::main]
            pub async fn main() -> ::std::process::ExitCode {
                omnia::main::<#backends_ty, Hooks>(#main_options).await
            }

            /// Run one deployment through this runtime's hosts and backends,
            /// blocking until the guest completes.
            #[tokio::main]
            pub async fn run(builder: omnia::DeploymentBuilder) -> Result<omnia::ExitStatus> {
                omnia::run::<#backends_ty, Hooks>(builder.mode(#mode)).await
            }
        }

        #[allow(unused_imports)]
        pub use runtime::{run, main};
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

    #[test]
    fn expand_config_file() {
        insta::assert_snapshot!(expand_pretty(quote!({
            config: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),
            hosts: {
                WasiOtel: OtelDefault,
            },
        })));
    }

    #[test]
    fn expand_resolver() {
        insta::assert_snapshot!(expand_pretty(quote!({
            guests: [
                { id: "api", source: "api.wasm" },
            ],
            resolver: CacheResolver::new(),
            hosts: {
                WasiOtel: OtelDefault,
            },
        })));
    }

    #[test]
    fn expand_command_guest() {
        insta::assert_snapshot!(expand_pretty(quote!({
            mode: command,
            guests: [
                { id: "app", source: "app.wasm" },
                { id: "helper", source: "helper.wasm" },
            ],
            command_guest: "app",
        })));
    }

    #[test]
    fn expand_program() {
        insta::assert_snapshot!(expand_pretty(quote!({
            mode: command,
            program: "mytool",
            config: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),
        })));
    }

    // The composed shape from the guest-resolution design: static guests plus
    // resolve-on-miss, explicit command routing, and raw argv passthrough.
    #[test]
    fn expand_deployment_keys() {
        insta::assert_snapshot!(expand_pretty(quote!({
            mode: command,
            program: "specify-example",
            guests: [
                { id: "specify", source: engine_component_path() },
                { id: "target:mock", source: mock_target_path() },
            ],
            mounts: [
                { name: "project", path: project_root(), writable: true },
                { name: "store", path: store_root(), writable: true },
            ],
            resolver: CacheResolver::new(),
            command_guest: "specify",
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
            }
        })));
    }

    // A bytes-valued `source:` (the `include_bytes!` embedding shape) passes
    // through to `GuestEntry::new` unchanged.
    #[test]
    fn expand_embedded_bytes() {
        insta::assert_snapshot!(expand_pretty(quote!({
            guests: [
                {
                    id: "specify",
                    source: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/specify.wasm")),
                },
            ],
            hosts: {
                WasiOtel: OtelDefault,
            },
        })));
    }

    #[test]
    fn expand_inline_manifest() {
        insta::assert_snapshot!(expand_pretty(quote!({
            guests: [
                {
                    id: "responder",
                    source: concat!(env!("CARGO_MANIFEST_DIR"), "/responder.wasm"),
                },
                {
                    id: "router",
                    source: concat!(env!("CARGO_MANIFEST_DIR"), "/router.wasm"),
                    link: ["omnia:link/echo"],
                },
            ],
            link: ["omnia:link/other"],
            mounts: [
                { name: ".", path: concat!(env!("CARGO_MANIFEST_DIR"), "/workspace"), writable: true },
            ],
            routes: {
                http: [{ prefix: "/", guest: "router" }],
                messaging: [{ topic: "orders.>", guest: "worker" }],
                websocket: [{ route: "chat.*", guest: "ws" }],
            },
            hosts: {
                WasiOtel: OtelDefault,
            },
        })));
    }
}
