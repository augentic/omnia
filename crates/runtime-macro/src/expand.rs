//! # Runtime macro expansion
//!
//! Expands the parsed runtime configuration into a complete runtime implementation.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Path};

use crate::runtime::Config;

// Generate the runtime from the configuration.
// A single cohesive code-generation function; its length is inherent.
#[allow(clippy::too_many_lines)]
pub fn expand(config: &Config) -> syn::Result<TokenStream> {
    let Expanded {
        context_fields,
        backend_idents,
        backend_types,
        store_ctx_fields,
        store_ctx_values,
        host_trait_impls,
        server_trait_impls,
        wasi_view_impls,
        main_fn,
    } = Expanded::try_from(config)?;

    // Connect every backend concurrently. `tokio::try_join!` is variadic and
    // returns on the first error, but rejects an empty argument list, so skip it
    // entirely when there are no backends.
    let connect_backends = if backend_idents.is_empty() {
        quote! {}
    } else {
        quote! {
            let (#(#backend_idents,)*) = tokio::try_join!(
                #(<#backend_types as Backend>::connect(),)*
            )?;
        }
    };

    Ok(quote! {
        mod runtime {
            use std::path::PathBuf;
            use std::sync::Arc;

            use anyhow::Result;
            use omnia::anyhow::Context as _;
            use omnia::futures::future::{try_join_all, BoxFuture};
            use omnia::tokio;
            use omnia::wasmtime::component::HasData;
            use omnia::wasmtime::{StoreLimits, StoreLimitsBuilder};
            use omnia::wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
            use omnia::{Backend, Compiled, Registry, HasLimits, RuntimeOptions, Runtime, Server};

            use super::*;

            /// Run a guest (single-file shorthand) or a manifest-driven deployment.
            pub async fn run(wasm: Option<PathBuf>, config: Option<PathBuf>) -> Result<()> {
                let compiled = omnia::RegistryBuilder::new()
                    .wasm(wasm)
                    .config(config)
                    .compile::<StoreCtx>()
                    .context("building runtime")?;
                let run_state = Context::new(compiled)
                    .await
                    .context("preparing runtime state")?;
                run_state.start().await.context("starting runtime services")
            }

            /// Initiator state holding the guest registry and backend connections.
            #[derive(Clone)]
            struct Context {
                registry: Arc<Registry<StoreCtx>>,
                #(pub #context_fields,)*
            }

            impl Context {
                /// Creates a new runtime state by linking WASI interfaces and connecting to backends.
                async fn new(mut compiled: Compiled<StoreCtx>) -> Result<Self> {
                    // link enabled WASI components
                    #(compiled.link::<#host_trait_impls>()?;)*

                    // connect to all backends concurrently
                    #connect_backends

                    // Pre-instantiate every guest against the now fully-linked
                    // linker and assemble the registry.
                    Ok(Self {
                        registry: Arc::new(compiled.build()?),
                        #(#backend_idents,)*
                    })
                }

                /// Start servers.
                ///
                /// N.B. for simplicity, all hosts are "servers" with a default implementation that
                /// does nothing.
                async fn start(&self) -> Result<()> {
                    // Drive epoch interruption so guest deadlines (and the
                    // wall-clock timeouts wrapped around each invocation) fire
                    // even while a guest executes CPU-bound code.
                    omnia::drive_epoch(
                        self.registry.engine().clone(),
                        self.registry.options().epoch_tick,
                    );

                    // Periodically sample pool occupancy as metrics so pool sizing can be tuned
                    // from real data.
                    omnia::sample_pool(
                        self.registry.engine().clone(),
                        self.registry.options().pool_metrics_interval,
                    );

                    // Wire the serve side of any host-mediated links before
                    // triggers fire, so a dispatched call always finds its
                    // target's wRPC server. A no-op when no `link`s are declared.
                    omnia::serve_links(self)
                        .await
                        .context("wiring host-mediated link serve side")?;

                    // Every server runs against the same `self`, so they share one
                    // registry and therefore one `Engine`. The pooling allocator's
                    // pool is per-`Engine`, so this keeps all per-request
                    // instantiation drawing from one shared pool.
                    let futures: Vec<BoxFuture<'_, Result<()>>> =
                        vec![#(Box::pin(#server_trait_impls.run(self)),)*];
                    try_join_all(futures).await?;
                    Ok(())
                }
            }

            impl Runtime for Context {
                type StoreCtx = StoreCtx;

                fn registry(&self) -> &Registry<Self::StoreCtx> {
                    &self.registry
                }

                fn options(&self) -> &RuntimeOptions {
                    self.registry.options()
                }

                fn store(&self) -> Self::StoreCtx {
                    let wasi_ctx = WasiCtxBuilder::new()
                        // .inherit_args()
                        .inherit_env()
                        .inherit_stdin()
                        .stdout(tokio::io::stdout())
                        .stderr(tokio::io::stderr())
                        .build();

                    StoreCtx {
                        table: ResourceTable::new(),
                        wasi: wasi_ctx,
                        limits: StoreLimitsBuilder::new()
                            .memory_size(self.registry.options().max_memory_bytes)
                            .build(),
                        // Per-store wRPC view state for host-mediated dynamic
                        // linking; inert unless the deployment declares `link`s.
                        wrpc: omnia::WrpcState::new(),
                        // Type-erased host→guest dispatcher (e.g. `wasi-model`'s
                        // `resolve`); a fresh handle to this `Context`. Inert
                        // unless a host binding reaches for it.
                        host_dispatch: Arc::new(self.clone()),
                        #(#store_ctx_values,)*
                    }
                }
            }

            /// Per-guest instance data shared between the runtime and the guest.
            pub struct StoreCtx {
                pub table: ResourceTable,
                pub wasi: WasiCtx,
                pub limits: StoreLimits,
                pub wrpc: omnia::WrpcState,
                /// Type-erased host→guest dispatcher backing host-mediated calls
                /// such as `wasi-model`'s `resolve`. Inert unless a host uses it.
                pub host_dispatch: Arc<dyn omnia::HostDispatch>,
                #(pub #store_ctx_fields,)*
            }

            /// WASI view implementation for the default WASI context.
            impl WasiView for StoreCtx {
                fn ctx(&mut self) -> WasiCtxView<'_> {
                    WasiCtxView {
                        ctx: &mut self.wasi,
                        table: &mut self.table,
                    }
                }
            }

            /// Exposes per-guest resource limits to the runtime.
            impl HasLimits for StoreCtx {
                fn limits(&mut self) -> &mut StoreLimits {
                    &mut self.limits
                }
            }

            /// wRPC view implementation backing host-mediated dynamic linking:
            /// every store can encode/serve linked calls over the bound carrier.
            impl omnia::WrpcView for StoreCtx {
                type Invoke = omnia::LinkClient;

                fn wrpc(&mut self) -> omnia::WrpcCtxView<'_, omnia::LinkClient> {
                    self.wrpc.view(&mut self.table)
                }
            }

            // WASI view implementations for enabled hosts.
            #(#wasi_view_impls)*
        }

        // Main function (optional)
        #main_fn
    })
}

struct Expanded {
    context_fields: Vec<TokenStream>,
    backend_idents: Vec<Ident>,
    backend_types: Vec<Path>,
    store_ctx_fields: Vec<TokenStream>,
    store_ctx_values: Vec<TokenStream>,
    host_trait_impls: Vec<Path>,
    server_trait_impls: Vec<TokenStream>,
    wasi_view_impls: Vec<TokenStream>,
    main_fn: TokenStream,
}

impl TryFrom<&Config> for Expanded {
    type Error = syn::Error;

    fn try_from(input: &Config) -> Result<Self, Self::Error> {
        // `Context` struct
        let mut context_fields = Vec::new();
        let mut backend_idents = Vec::new();
        let mut backend_types = Vec::new();
        let mut seen_backends = Vec::new();

        for backend in &input.backends {
            // deduplicate backends based on their string representation
            let backend_str = quote! {#backend}.to_string();
            if seen_backends.contains(&backend_str) {
                continue;
            }
            seen_backends.push(backend_str);

            let field = field_ident(backend);
            context_fields.push(quote! {#field: #backend});
            backend_idents.push(field);
            backend_types.push(backend.clone());
        }

        let mut store_ctx_fields = Vec::new();
        let mut store_ctx_values = Vec::new();
        let mut host_trait_impls = Vec::new();
        let mut server_trait_impls = Vec::new();
        let mut wasi_view_impls = Vec::new();

        for host in &input.hosts {
            let host_type = &host.type_;
            let host_ident = wasi_ident(host_type);
            let backend_type = &host.backend;
            let backend_ident = field_ident(backend_type);

            host_trait_impls.push(host_type.clone());
            store_ctx_fields.push(quote! {#host_ident: #backend_type});
            store_ctx_values.push(quote! {#host_ident: self.#backend_ident.clone()});

            // servers
            server_trait_impls.push(quote! {#host_type});

            // WASI view impls
            // HACK: derive module name from WASI type
            let module = wasi_ident(host_type);
            wasi_view_impls.push(quote! {
                #module::omnia_wasi_view!(StoreCtx, #host_ident);
            });
        }

        // main function (optional)
        let main_fn = if input.gen_main {
            quote! {
                use omnia::tokio;

                #[tokio::main]
                async fn main() -> anyhow::Result<()> {
                    use omnia::Parser;
                    match omnia::Cli::parse().command {
                        omnia::Command::Run { wasm, config } => runtime::run(wasm, config).await,
                        _ => unreachable!(),
                    }
                }
            }
        } else {
            quote! {}
        };

        Ok(Self {
            context_fields,
            backend_idents,
            backend_types,
            store_ctx_fields,
            store_ctx_values,
            host_trait_impls,
            server_trait_impls,
            wasi_view_impls,
            main_fn,
        })
    }
}

/// Generates a field name for a backend type.
fn field_ident(path: &Path) -> Ident {
    let Some(ident) = path.segments.last() else {
        return format_ident!("field");
    };
    let ident_str = quote! {#ident}.to_string();

    // convert the type string to snake_case
    let mut field_str = String::new();
    for char in ident_str.chars() {
        if char.is_uppercase() {
            if !field_str.is_empty() {
                field_str.push('_');
            }
            field_str.push_str(&char.to_lowercase().to_string());
        } else {
            field_str.push(char);
        }
    }

    format_ident!("{field_str}")
}

fn wasi_ident(path: &Path) -> Ident {
    let Some(ident) = path.segments.last() else {
        return format_ident!("wasi");
    };

    let name = quote! {#ident}.to_string();
    let name = name.replace("Wasi", "omnia_wasi_").to_lowercase();
    format_ident!("{name}")
}
