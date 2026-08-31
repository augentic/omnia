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
        acquirer_hook,
        link_plugins,
    } = Codegen::from(config);

    let mode = mode.tokens();
    // A declared `plugins:` block opts the deployment into the loader host;
    // worlds that do not import `omnia:plugins/loader` never see it.
    let plugins_host = link_plugins.then(|| {
        quote! {
            deployment.host::<omnia::WasiPlugins, #backends_ty>()?;
        }
    });

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
                    #plugins_host
                    #(deployment.host::<#host_types, #backends_ty>()?;)*
                    Ok(())
                }

                #acquirer_hook

                async fn serve(
                    runtime: &omnia::Runtime<#backends_ty>,
                ) -> Result<()> {
                    // Every host runs uniformly: capability hosts resolve
                    // immediately through `Server`'s no-op default, trigger
                    // servers loop until shutdown.
                    let servers: Vec<future::BoxFuture<'_, Result<()>>> = vec![
                        #(
                            Box::pin(#host_types.run(runtime)),
                        )*
                    ];
                    future::try_join_all(servers).await?;
                    Ok(())
                }
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
                omnia::run::<#backends_ty, Hooks>(builder.mode(#mode)).await
            }
        }

        #[allow(unused_imports)]
        pub use runtime::{run, main};
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

    // A backend shared by non-adjacent hosts must emit exactly one struct
    // field (interleaved duplicates defeat a consecutive-only dedup).
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
    fn expand_config_file() {
        insta::assert_snapshot!(expand_pretty(quote!({
            config: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),
            hosts: {
                WasiOtel: OtelDefault,
            },
        })));
    }

    // A `command: true` guest entry marks the command-mode target; the flag
    // expands to `.command()` on its `GuestEntry`.
    #[test]
    fn expand_command_flag() {
        insta::assert_snapshot!(expand_pretty(quote!({
            mode: command,
            guests: [
                { id: "app", source: "app.wasm", command: true },
                { id: "helper", source: "helper.wasm" },
            ],
        })));
    }

    // The composed deployment shape: static guests, mounts, and explicit
    // command routing.
    #[test]
    fn expand_deployment_keys() {
        insta::assert_snapshot!(expand_pretty(quote!({
            mode: command,
            guests: [
                { id: "specify", source: engine_component_path(), command: true },
                { id: "target:mock", source: mock_target_path() },
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

    // Guest-owned routes and the deployment-wide plugins block: every trigger
    // list expands to `route_*` builder calls on the owning `GuestEntry` (the
    // guest id is the implicit target), the `plugins:` block's `interfaces:`
    // list to `.plugins(...)` calls on the `Manifest` plus the `WasiPlugins`
    // loader host link, and patterns/interfaces are arbitrary expressions.
    #[test]
    fn expand_inline_manifest() {
        insta::assert_snapshot!(expand_pretty(quote!({
            plugins: { interfaces: ["omnia:link/echo"] },
            guests: [
                {
                    id: "responder",
                    source: concat!(env!("CARGO_MANIFEST_DIR"), "/responder.wasm"),
                    routes: {
                        messaging: ["orders.>"],
                        websocket: ["chat.*"],
                    },
                },
                {
                    id: "router",
                    source: concat!(env!("CARGO_MANIFEST_DIR"), "/router.wasm"),
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

    // The full plugins block: `interfaces:` reaches the manifest, `acquire:`
    // lowers into the generated `Wiring::acquirer` hook, and the
    // `WasiPlugins` loader host is linked.
    #[test]
    fn expand_plugins_acquire() {
        insta::assert_snapshot!(expand_pretty(quote!({
            plugins: {
                interfaces: ["emery:adapter/probe"],
                acquire: omnia::MountAcquire,
            },
            guests: [
                { id: "engine", source: "engine.wasm" },
            ],
            mounts: [
                { name: ".", path: project_root() },
            ],
        })));
    }

    // An acquire-only plugins block carries no manifest data, so it composes
    // with `config:` — the TOML declares the interfaces, the binary the
    // acquirer.
    #[test]
    fn expand_plugins_acquire_with_config() {
        insta::assert_snapshot!(expand_pretty(quote!({
            config: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),
            plugins: { acquire: omnia::MountAcquire },
        })));
    }

    // The declarative locations grammar, cached: path entries fold in
    // declaration order into one `PathAcquire`, the registry entry becomes a
    // `RegistryAcquire` cached in the `cache:` backend — which joins the
    // bundle beside the hosts' backends — and the two compose by location
    // kind in the generated `Wiring::acquirer` hook.
    #[test]
    fn expand_locations_cached() {
        insta::assert_snapshot!(expand_pretty(quote!({
            plugins: {
                interfaces: ["emery:adapter/probe"],
                locations: [
                    { name: ".", path: project_root() },
                    { registry: "ghcr.io" },
                ],
                cache: PluginCache,
            },
            guests: [
                { id: "engine", source: "engine.wasm" },
            ],
            hosts: {
                WasiOtel: OtelDefault,
            },
        })));
    }

    // Cacheless locations: no store backend joins the bundle and the
    // registry acquirer fetches fresh on every load.
    #[test]
    fn expand_locations_cacheless() {
        insta::assert_snapshot!(expand_pretty(quote!({
            plugins: {
                interfaces: ["emery:adapter/probe"],
                locations: [
                    { name: "adapters", path: adapters_root() },
                    { registry: "ghcr.io" },
                ],
            },
            guests: [
                { id: "engine", source: "engine.wasm" },
            ],
        })));
    }

    // Locations are acquisition policy, not manifest data, so the block
    // composes with `config:` — the TOML declares the interfaces, the
    // binary the locations and cache.
    #[test]
    fn expand_locations_with_config() {
        insta::assert_snapshot!(expand_pretty(quote!({
            config: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),
            plugins: {
                locations: [{ registry: "ghcr.io" }],
                cache: PluginCache,
            },
        })));
    }
}
