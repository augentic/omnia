//! # Runtime macro expansion
//!
//! Expands the parsed runtime configuration into a complete runtime implementation.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Path};

use crate::runtime::Config;

// Generate the runtime from the configuration.
pub fn expand(config: &Config) -> syn::Result<TokenStream> {
    let Expanded {
        context_fields,
        store_ctx_fields,
        store_ctx_values,
        host_trait_impls,
        server_trait_impls,
        wasi_view_impls,
        main_fn,
    } = Expanded::try_from(config)?;

    Ok(quote! {
        mod runtime {
            use std::path::PathBuf;

            use anyhow::Result;
            use qwasr::anyhow::Context as _;
            use qwasr::futures::future::{try_join_all, BoxFuture};
            use qwasr::tokio;
            use qwasr::wasmtime::component::{HasData,InstancePre};
            use qwasr::wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
            use qwasr::{Backend, Compiled, Server, State};

            use super::*;

            /// Run the specified wasm guest using the configured runtime.
            pub async fn run(wasm: PathBuf) -> Result<()> {
                let mut compiled = qwasr::create(&wasm)
                    .with_context(|| format!("compiling {}", wasm.display()))?;
                let run_state = Context::new(&mut compiled)
                    .await
                    .context("preparing runtime state")?;
                run_state.start().await.context("starting runtime services")
            }

            /// Initiator state holding pre-instantiated components and backend connections.
            #[derive(Clone)]
            struct Context {
                instance_pre: InstancePre<StoreCtx>,
                #(pub #context_fields,)*
            }

            impl Context {
                /// Creates a new runtime state by linking WASI interfaces and connecting to backends.
                async fn new(compiled: &mut Compiled<StoreCtx>) -> Result<Self> {
                    // link enabled WASI components
                    #(compiled.link(#host_trait_impls)?;)*

                    Ok(Self {
                        instance_pre: compiled.pre_instantiate()?,
                        #(#context_fields::connect().await?,)*
                    })
                }

                /// Start servers.
                ///
                /// N.B. for simplicity, all hosts are "servers" with a default implementation that does nothing.
                async fn start(&self) -> Result<()> {
                    let futures: Vec<BoxFuture<'_, Result<()>>> =
                        vec![#(Box::pin(#server_trait_impls.run(self)),)*];
                    try_join_all(futures).await?;
                    Ok(())
                }
            }

            impl State for Context {
                type StoreCtx = StoreCtx;

                fn instance_pre(&self) -> &InstancePre<Self::StoreCtx> {
                    &self.instance_pre
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
                        #(#store_ctx_values,)*
                    }
                }
            }

            /// Per-guest instance data shared between the runtime and the guest.
            pub struct StoreCtx {
                pub table: ResourceTable,
                pub wasi: WasiCtx,
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

            // WASI view implementations for enabled hosts.
            #(#wasi_view_impls)*
        }

        // Main function (optional)
        #main_fn
    })
}

struct Expanded {
    context_fields: Vec<TokenStream>,
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
                #module::qwasr_wasi_view!(StoreCtx, #host_ident);
            });
        }

        // main function (optional)
        let main_fn = if input.gen_main {
            quote! {
                use qwasr::tokio;

                #[tokio::main]
                async fn main() -> anyhow::Result<()> {
                    use qwasr::Parser;
                    match qwasr::Cli::parse().command {
                        qwasr::Command::Run { wasm } => runtime::run(wasm).await,
                        _ => unreachable!(),
                    }
                }
            }
        } else {
            quote! {}
        };

        Ok(Self {
            context_fields,
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
    let name = name.replace("Wasi", "qwasr_wasi_").to_lowercase();
    format_ident!("{name}")
}
