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
        host_trait_impls,
        command_guard,
        run_fn,
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
                // Guest args (`args[0]` = program name). Empty for servers.
                #[runtime(args)]
                args: Arc<Vec<String>>,
                // Working-tree contains startup-validated preopens. Empty unless the deployment
                // configures `[[mount]]`s or sets `OMNIA_WORKING_TREE`.
                #[runtime(preopens)]
                working_trees: Arc<WorkingTreeRegistry>,
                #(#context_fields,)*
            }

            impl Context {
                /// Creates a new runtime state by linking WASI interfaces and connecting to backends.
                async fn new(
                    mut compiled: Compiled<StoreCtx>, args: Arc<Vec<String>>,
                ) -> Result<Self> {
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


            #run_fn
        }

        #main_fn
    })
}

struct Expanded {
    context_fields: Vec<TokenStream>,
    backend_idents: Vec<Ident>,
    backend_types: Vec<Path>,
    store_ctx_fields: Vec<TokenStream>,
    host_trait_impls: Vec<Path>,
    command_guard: TokenStream,
    run_fn: TokenStream,
    main_fn: TokenStream,
}

impl TryFrom<&Config> for Expanded {
    type Error = syn::Error;

    fn try_from(input: &Config) -> Result<Self, Self::Error> {
        let mut store_ctx_fields = Vec::new();
        let mut host_trait_impls = Vec::new();

        // For each backend field, the `StoreCtx` field names that clone from it,
        // keyed by backend field ident in host-declaration order. Emitted as
        // `#[runtime(store = ...)]` on the `Context` backend field so the
        // `Runtime` derive generates the matching `store()` assignment.
        let mut store_targets: Vec<(Ident, Vec<Ident>)> = Vec::new();

        for host in &input.hosts {
            let host_type = &host.type_;
            host_trait_impls.push(host_type.clone());

            // A backend-less host contributes no `StoreCtx` backend field or
            // host view; it only links its interface.
            if let Some(backend_type) = &host.backend {
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

                // Record the StoreCtx target this backend field feeds; several
                // hosts may share one (deduplicated) backend.
                if let Some((_, targets)) =
                    store_targets.iter_mut().find(|(backend, _)| *backend == backend_ident)
                {
                    targets.push(host_ident);
                } else {
                    store_targets.push((backend_ident, vec![host_ident]));
                }
            }
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
                .iter()
                .find(|(other, _)| *other == field)
                .into_iter()
                .flat_map(|(_, targets)| targets)
                .map(|target| quote! { #[runtime(store = #target)] })
                .collect();

            context_fields.push(quote! {
                #(#store_attrs)*
                pub #field: #backend
            });
            backend_idents.push(field);
            backend_types.push(backend.clone());
        }

        // `run` differs for command vs server deployments.
        let run_fn = run_fn(input.command, &host_trait_impls);

        let main_fn = if input.gen_main {
            main_fn()
        } else {
            quote! {}
        };

        Ok(Self {
            context_fields,
            backend_idents,
            backend_types,
            store_ctx_fields,
            host_trait_impls,
            command_guard,
            run_fn,
            main_fn,
        })
    }
}

/// The deployment's `run`: compile the registry and build runtime state, then
/// either drive the one-shot `wasi:cli` command (returning the guest's exit
/// status) or serve every long-lived trigger to completion.
fn run_fn(command: bool, hosts: &[Path]) -> TokenStream {
    let run_body = if command {
        quote! {
            // argv[0] is conventionally the program name (`wasmtime` does the
            // same); the guest reads it via `wasi:cli/environment`.
            let program = wasm
                .as_deref()
                .and_then(::std::path::Path::file_stem)
                .map_or_else(|| String::from("guest"), |stem| stem.to_string_lossy().into_owned());

            let compiled = omnia::RegistryBuilder::new()
                .wasm(wasm)
                .config(config)
                .compile::<StoreCtx>()
                .await
                .context("building runtime")?;

            let mut argv = Vec::with_capacity(args.len() + 1);
            argv.push(program);
            argv.extend(args);
            let run_state = Context::new(compiled, Arc::new(argv))
                .await
                .context("preparing runtime state")?;

            omnia::run_command(&run_state).await.context("running command")
        }
    } else {
        quote! {
            let compiled = omnia::RegistryBuilder::new()
                .wasm(wasm)
                .config(config)
                .compile::<StoreCtx>()
                .await
                .context("building runtime")?;
            let run_state = Context::new(compiled, Arc::new(args))
                .await
                .context("preparing runtime state")?;

            let servers: Vec<BoxFuture<'_, Result<()>>> =
                vec![#(Box::pin(#hosts.run(&run_state)),)*];
            omnia::serve(&run_state, servers).await.context("starting runtime services")?;

            Ok(omnia::ExitStatus::SUCCESS)
        }
    };

    quote! {
        /// Run a guest (single-file shorthand) or a manifest-driven deployment.
        pub async fn run(
            wasm: Option<PathBuf>, config: Option<PathBuf>, args: Vec<String>,
        ) -> Result<omnia::ExitStatus> {
            #run_body
        }
    }
}

/// The runtime entrypoint: parse the CLI, run to completion, and exit with the
/// guest's status (or a failure on a host error).
///
/// Both runtime shapes share this `main`: `run` always yields an
/// `ExitStatus` — the guest's for a one-shot command, `SUCCESS` for a
/// long-lived server's clean shutdown.
fn main_fn() -> TokenStream {
    quote! {
        use omnia::tokio;

        #[tokio::main]
        async fn main() -> ::std::process::ExitCode {
            use omnia::Parser;
            match omnia::Cli::parse().command {
                omnia::Command::Run { wasm, config, args } => {
                    match runtime::run(wasm, config, args).await {
                        Ok(status) => status.into(),
                        Err(error) => {
                            eprintln!("{error:#}");
                            ::std::process::ExitCode::FAILURE
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
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
