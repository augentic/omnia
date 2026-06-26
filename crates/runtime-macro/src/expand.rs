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

    // `tokio` is only referenced by `try_join!` above, so import it only when
    // there is at least one backend to connect (avoids an unused import).
    let tokio_import = if backend_idents.is_empty() {
        quote! {}
    } else {
        quote! { use omnia::tokio; }
    };

    Ok(quote! {
        mod runtime {
            use std::path::PathBuf;
            use std::sync::Arc;

            use anyhow::Result;
            use omnia::anyhow::Context as _;
            use omnia::futures::future::BoxFuture;
            #tokio_import
            use omnia::{Backend, Compiled, Registry, Runtime, Server, StoreBase, StoreContext};

            use super::*;

            /// Run a guest (single-file shorthand) or a manifest-driven deployment.
            pub async fn run(wasm: Option<PathBuf>, config: Option<PathBuf>) -> Result<()> {
                let compiled = omnia::RegistryBuilder::new()
                    .wasm(wasm)
                    .config(config)
                    .compile::<StoreCtx>()
                    .await
                    .context("building runtime")?;
                let run_state = Context::new(compiled)
                    .await
                    .context("preparing runtime state")?;

                // Every server runs against the same `run_state`, so they share
                // one registry and therefore one `Engine` (and its pooling
                // allocator's pool). `omnia::serve` drives epoch interruption,
                // pool-metric sampling, and the host-mediated link serve side
                // around them.
                let servers: Vec<BoxFuture<'_, Result<()>>> =
                    vec![#(Box::pin(#server_trait_impls.run(&run_state)),)*];
                omnia::serve(&run_state, servers).await.context("starting runtime services")
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
                    #(compiled.host::<#host_trait_impls>()?;)*

                    // connect to all backends concurrently
                    #connect_backends

                    // Pre-instantiate every guest against the now fully-linked
                    // linker and assemble the registry.
                    Ok(Self {
                        registry: Arc::new(compiled.build()?),
                        #(#backend_idents,)*
                    })
                }
            }

            impl Runtime for Context {
                type StoreCtx = StoreCtx;

                fn registry(&self) -> &Registry<Self::StoreCtx> {
                    &self.registry
                }

                fn store(&self) -> Self::StoreCtx {
                    StoreCtx {
                        // Fixed per-store state: WASI inheritance, the memory
                        // limiter, inert wRPC view state, and a fresh host->guest
                        // dispatch handle to this `Context`.
                        base: StoreBase::new(self.options(), Arc::new(self.clone())),
                        #(#store_ctx_values,)*
                    }
                }
            }

            /// Per-guest instance data shared between the runtime and the guest.
            ///
            /// The `StoreContext` derive implements `WasiView`, `WrpcView`, and
            /// `HasLimits` against `base`, plus one host view per `#[wasi(...)]`
            /// backend field.
            #[derive(StoreContext)]
            pub struct StoreCtx {
                #[base]
                pub base: StoreBase,
                #(#store_ctx_fields,)*
            }
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

        for host in &input.hosts {
            let host_type = &host.type_;
            // The host's `StoreCtx` field name and host-crate module path
            // coincide (e.g. `WasiHttp` -> `omnia_wasi_http`), so the
            // `StoreContext` derive's `#[wasi(omnia_wasi_http)]` attribute emits
            // `omnia_wasi_http::omnia_wasi_view!(StoreCtx, omnia_wasi_http)`.
            let host_ident = wasi_ident(host_type);
            let backend_type = &host.backend;
            let backend_ident = field_ident(backend_type);

            host_trait_impls.push(host_type.clone());
            store_ctx_fields.push(quote! {
                #[wasi(#host_ident)]
                pub #host_ident: #backend_type
            });
            store_ctx_values.push(quote! {#host_ident: self.#backend_ident.clone()});

            // servers
            server_trait_impls.push(quote! {#host_type});
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
