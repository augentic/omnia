//! # Runtime macro expansion
//!
//! Expands the parsed runtime configuration into a complete runtime implementation.

use std::collections::BTreeMap;

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
        host_trait_impls,
        command_guard,
    } = Expanded::try_from(config)?;

    let command = config.command;
    let servers = if command {
        quote! { vec![] }
    } else {
        quote! { vec![#(Box::pin(#host_trait_impls.run(&run_state)),)*] }
    };

    let body = quote! {
        let compiled = omnia::RegistryBuilder::new()
            .wasm(wasm)
            .config(config)
            .compile::<StoreCtx>()
            .await
            .context("building runtime")?;
        let run_state = Context::new(compiled, Arc::new(args), #command)
            .await
            .context("preparing runtime state")?;
        omnia::drive(&run_state, #command, #servers)
            .await
            .context("running deployment")
    };

    let connect_backends = connect_backends(&backend_idents, &backend_types);

    Ok(quote! {
        mod runtime {
            use std::path::PathBuf;
            use std::sync::Arc;

            use anyhow::Result;
            use omnia::anyhow::Context as _;
            use omnia::tokio;
            use omnia::{
                Backend, Compiled, Registry, Runtime, Server, StoreBase, StoreContext,
                WorkingTreeRegistry,
            };

            use super::*;

            #command_guard

            // Runtime state holding the guest registry and backend connections.
            #[derive(Clone, Runtime)]
            #[runtime(store = StoreCtx)]
            struct Context {
                #[runtime(registry)]
                registry: Arc<Registry<StoreCtx>>,
                // Guest args. `args[0]` = program name when CLI command.
                #[runtime(args)]
                args: Arc<Vec<String>>,
                // Working-tree contains startup-validated preopens when the deployment
                // configures `[[mount]]`s or sets `OMNIA_WORKING_TREE`.
                #[runtime(preopens)]
                working_trees: Arc<WorkingTreeRegistry>,
                #(#context_fields,)*
            }

            impl Context {
                /// Creates a new runtime state by linking WASI interfaces and connecting to backends.
                async fn new(
                    mut compiled: Compiled<StoreCtx>, args: Arc<Vec<String>>, command: bool,
                ) -> Result<Self> {
                    let args = Arc::new(if command {
                        compiled.argv((*args).clone())
                    } else {
                        (*args).clone()
                    });

                    // link enabled WASI components
                    #(compiled.host::<#host_trait_impls>()?;)*

                    // connect to all backends concurrently
                    #connect_backends

                    // snapshot the startup-validated working-tree
                    let working_trees = compiled.working_trees();

                    // build the store context
                    Ok(Self {
                        registry: Arc::new(compiled.build()?),
                        args,
                        working_trees,
                        #(#backend_idents,)*
                    })
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

            /// Build runtime state from the parsed CLI inputs and drive the
            /// deployment to the guest's exit status (or a host error).
            async fn run(
                wasm: Option<PathBuf>, config: Option<PathBuf>, args: Vec<String>,
            ) -> Result<omnia::ExitStatus> {
                #body
            }

            /// Parse the CLI and drive the deployment to a process exit code: the
            /// guest's status for a one-shot `command`, success for a long-lived
            /// server's clean shutdown, or failure on a host error.
            #[tokio::main]
            pub async fn main() -> ::std::process::ExitCode {
                omnia::run_main(run).await
            }
        }

        use runtime::main;
    })
}

struct Expanded {
    context_fields: Vec<TokenStream>,
    backend_idents: Vec<Ident>,
    backend_types: Vec<Path>,
    store_ctx_fields: Vec<TokenStream>,
    host_trait_impls: Vec<Path>,
    command_guard: TokenStream,
}

impl TryFrom<&Config> for Expanded {
    type Error = syn::Error;

    fn try_from(input: &Config) -> Result<Self, Self::Error> {
        let mut store_ctx_fields = Vec::new();
        let mut host_trait_impls = Vec::new();
        let mut store_targets: BTreeMap<String, Vec<Ident>> = BTreeMap::new();

        for host in &input.hosts {
            let host_type = &host.type_;
            host_trait_impls.push(host_type.clone());

            // A backend-less host contributes no `StoreCtx` backend field or
            // host view; it only links its interface.
            let Some(backend_type) = &host.backend else {
                continue;
            };

            // The host's `StoreCtx` field name and host-crate module path
            // coincide (e.g. `WasiHttp` -> `omnia_wasi_http`), so the
            // `StoreContext` derive's `#[wasi(omnia_wasi_http)]` attribute
            // emits `omnia_wasi_http::omnia_wasi_view!(StoreCtx, …)`.
            let host_ident = wasi_ident(host_type);
            let backend_ident = field_ident(backend_type);
            store_ctx_fields.push(quote! {
                #[wasi(#host_ident)]
                pub #host_ident: #backend_type
            });
            store_targets.entry(backend_ident.to_string()).or_default().push(host_ident);
        }

        // Guardrail: a command deployment cannot link a long-lived trigger
        // server (it runs once and exits). Each host's `IS_SERVER` flag is read
        // from the type system, so a newly added trigger is covered without
        // touching this macro.
        let command_guard = if input.command {
            quote! {
                const _: () = omnia::assert_command_hosts(&[
                    #(<#host_trait_impls as Server<Context>>::IS_SERVER,)*
                ]);
            }
        } else {
            quote! {}
        };

        let mut context_fields = Vec::new();
        let mut backend_idents = Vec::new();
        let mut backend_types = Vec::new();

        for backend in &input.backends {
            let field = field_ident(backend);
            let store_attrs: Vec<TokenStream> = store_targets
                .get(&field.to_string())
                .into_iter()
                .flatten()
                .map(|target| quote! { #[runtime(store = #target)] })
                .collect();

            context_fields.push(quote! {
                #(#store_attrs)*
                pub #field: #backend
            });
            backend_idents.push(field);
            backend_types.push(backend.clone());
        }

        Ok(Self {
            context_fields,
            backend_idents,
            backend_types,
            store_ctx_fields,
            host_trait_impls,
            command_guard,
        })
    }
}

fn connect_backends(backend_idents: &[Ident], backend_types: &[Path]) -> TokenStream {
    if backend_idents.is_empty() {
        quote! {}
    } else {
        quote! {
            let (#(#backend_idents,)*) = tokio::try_join!(
                #(<#backend_types as Backend>::connect(),)*
            )?;
        }
    }
}

/// Derives a `snake_case` field name from a backend type's final path segment
/// (e.g. `HttpDefault` -> `http_default`).
fn field_ident(path: &Path) -> Ident {
    let Some(segment) = path.segments.last() else {
        return format_ident!("field");
    };

    let mut snake = String::new();
    for ch in segment.ident.to_string().chars() {
        if ch.is_uppercase() {
            if !snake.is_empty() {
                snake.push('_');
            }
            snake.extend(ch.to_lowercase());
        } else {
            snake.push(ch);
        }
    }

    format_ident!("{snake}")
}

/// Derives a host crate's module ident from a host type's final path segment
/// (e.g. `WasiHttp` -> `omnia_wasi_http`), matching the host-crate naming the
/// `StoreContext` derive's `#[wasi(...)]` attribute expects.
fn wasi_ident(path: &Path) -> Ident {
    let Some(segment) = path.segments.last() else {
        return format_ident!("wasi");
    };

    let name = segment.ident.to_string().replace("Wasi", "omnia_wasi_").to_lowercase();
    format_ident!("{name}")
}
