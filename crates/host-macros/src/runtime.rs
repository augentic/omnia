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
        ctx_keys,
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
                #(B: omnia::Provides<#ctx_keys>,)*
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

        #[allow(unused_imports)]
        pub use runtime::{Hooks, main, manifest, run, run_with};
    }
}

// Unit tests by design: macro token expansion is pure.
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

    // A `Backend(options)` row lowers to `connect_with(options)`; rows
    // sharing that backend ride the same compiled-in connection.
    #[test]
    fn expand_connect_options() {
        insta::assert_snapshot!(expand_pretty(quote!({
            hosts: {
                WasiKeyValue: Filesystem(FilesystemOptions::at(".omnia/storage")),
                WasiBlobstore: Filesystem(FilesystemOptions::at(".omnia/storage")),
                WasiOtel: OtelDefault,
            },
        })));
    }

    // interleaved duplicates defeat a consecutive-only dedup
    #[test]
    fn expand_shared_backend() {
        insta::assert_snapshot!(expand_pretty(quote!({
            hosts: {
                WasiKeyValue: Redis,
                WasiOtel: OtelDefault,
                WasiMessaging: Redis,
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
    fn expand_manifest_file() {
        insta::assert_snapshot!(expand_pretty(quote!({
            manifest: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),
            hosts: {
                WasiOtel: OtelDefault,
            },
        })));
    }

    // `command: true` expands to `.command()`; a guest without `name:` is named by its stem
    #[test]
    fn expand_command_flag() {
        insta::assert_snapshot!(expand_pretty(quote!({
            mode: command,
            guests: [
                { path: "app.wasm", command: true },
                { path: "helper.wasm" },
            ],
        })));
    }

    #[test]
    fn expand_deployment_keys() {
        insta::assert_snapshot!(expand_pretty(quote!({
            mode: command,
            guests: [
                { name: "specify", path: "engine.wasm", command: true },
                { name: "target:mock", path: "mock.wasm" },
            ],
            mounts: [
                { name: "project", path: project_root(), writable: true },
                { name: "store", path: store_root(), writable: true },
            ],
            hosts: {
                WasiHttp: HttpDefault,
                WasiOtel: OtelDefault,
            }
        })));
    }

    // a macro-valued `path:` passes through to `include_bytes!` unchanged
    #[test]
    fn expand_embedded_bytes() {
        insta::assert_snapshot!(expand_pretty(quote!({
            guests: [
                { path: concat!(env!("CARGO_MANIFEST_DIR"), "/specify.wasm") },
            ],
            hosts: {
                WasiOtel: OtelDefault,
            },
        })));
    }

    // every trigger list expands to `route_*` calls on the owning entry
    #[test]
    fn expand_routes() {
        insta::assert_snapshot!(expand_pretty(quote!({
            guests: [
                {
                    name: "responder",
                    path: concat!(env!("CARGO_MANIFEST_DIR"), "/responder.wasm"),
                    routes: {
                        messaging: ["orders.>"],
                        websocket: ["chat.*"],
                    },
                },
                {
                    name: "router",
                    path: concat!(env!("CARGO_MANIFEST_DIR"), "/router.wasm"),
                    routes: {
                        http: ["/", concat!("/", "api")],
                    },
                },
            ],
            mounts: [
                { name: ".", path: concat!(env!("CARGO_MANIFEST_DIR"), "/workspace"), writable: true },
            ],
            hosts: {
                WasiOtel: OtelDefault,
            },
        })));
    }

    // the expansion never names a loader path: assembly installs the registry client
    #[test]
    fn expand_guests_block() {
        insta::assert_snapshot!(expand_pretty(quote!({
            guests: [
                { path: "engine.wasm" },
            ],
            registries: include_str!("wasm-pkg.toml"),
            mounts: [
                { name: ".", path: project_root() },
            ],
            hosts: {
                WasiOtel: OtelDefault,
            },
        })));
    }

    // valid: the guests arrive at run time or over the cli
    #[test]
    fn expand_registries_only() {
        insta::assert_snapshot!(expand_pretty(quote!({
            registries: include_str!("wasm-pkg.toml"),
        })));
    }

    // an unnamed package is named by its reference without the version; a
    // `digest:` literal lands as the bytes it decoded to
    #[test]
    fn expand_package_defaults() {
        insta::assert_snapshot!(expand_pretty(quote!({
            guests: [
                { path: "app.wasm" },
                {
                    package: "acme:tool@1.2.3",
                    digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                },
                {
                    name: "pinned",
                    path: "pinned.wasm",
                    digest: "sha256:E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855",
                },
                { name: "tool", package: "acme:tool@2.0.0" },
            ],
            registries: include_str!("wasm-pkg.toml"),
        })));
    }

    #[test]
    fn digest_malformed() {
        for (literal, needle) in [
            ("md5:00", "missing `sha256:`"),
            ("sha256:abc", "got 3 hex characters"),
            (
                "sha256:zz00000000000000000000000000000000000000000000000000000000000000",
                "not hexadecimal",
            ),
        ] {
            let error = syn::parse2::<Config>(quote!({
                guests: [{ path: "app.wasm", digest: #literal }],
            }))
            .err()
            .expect("a malformed digest must be refused");
            assert!(error.to_string().contains(needle), "{error}");
        }
    }

    // A guest has one source; the retired `on_demand` and `wasm_only` keys
    // are unknown keys like any other.
    #[test]
    fn package_sources_exclusive() {
        let error = syn::parse2::<Config>(quote!({
            guests: [{ name: "tool", path: "tool.wasm", package: "acme:tool@1.2.3" }],
        }))
        .err()
        .expect("two sources must be refused");
        assert!(error.to_string().contains("mutually exclusive"), "{error}");

        for retired in ["on_demand", "wasm_only"] {
            let key = quote::format_ident!("{retired}");
            let error = syn::parse2::<Config>(quote!({
                guests: [{ path: "tool.wasm", #key: true }],
            }))
            .err()
            .expect("a retired key must be refused");
            assert!(
                error.to_string().contains(&format!("unknown guest key `{retired}`")),
                "{error}"
            );
        }
    }

    // `include_bytes!` takes a literal or a macro, so a path computed at run
    // time is refused where the key is named rather than deep in the expansion.
    #[test]
    fn guest_path_not_embeddable() {
        let error = syn::parse2::<Config>(quote!({
            guests: [{ path: engine_component_path() }],
        }))
        .err()
        .expect("a computed path must be refused");
        assert!(error.to_string().contains("string literal or a macro"), "{error}");
    }

    #[test]
    fn guest_path_missing() {
        let error = syn::parse2::<Config>(quote!({
            guests: [{ name: "api" }],
        }))
        .err()
        .expect("a guest without a path must be refused");
        assert!(error.to_string().contains("missing `path`"), "{error}");
    }

    // `registries` is manifest data, so it conflicts with `manifest:` like
    // every other inline key; the file declares `[registries]`.
    #[test]
    fn registries_refused_beside_manifest() {
        let error = syn::parse2::<Config>(quote!({
            manifest: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),
            registries: include_str!("wasm-pkg.toml"),
        }))
        .err()
        .expect("registries beside manifest must be refused");
        assert!(error.to_string().contains("mutually exclusive"), "{error}");
    }
}
