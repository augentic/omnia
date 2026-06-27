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
        colist_guard,
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
            use omnia::{Backend, Compiled, Registry, Runtime, Server, StoreBase, StoreContext};

            use super::*;

            #run_fn

            #colist_guard

            /// Initiator state holding the guest registry and backend connections.
            ///
            /// The `Runtime` derive generates `registry()` and `store()` from the
            /// `#[runtime(...)]` attributes: `store()` builds the fixed `base`
            /// (with the `#[runtime(args)]` argv) plus one cloned backend per
            /// `#[runtime(store = ...)]` field.
            #[derive(Clone, Runtime)]
            #[runtime(store = StoreCtx)]
            struct Context {
                #[runtime(registry)]
                registry: Arc<Registry<StoreCtx>>,
                /// Guest argv (`args[0]` is the program name), threaded into
                /// every store. Empty for a long-lived server; a `wasi:cli`
                /// command reads it as `wasi:cli/environment`.
                #[runtime(args)]
                args: Arc<Vec<String>>,
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

                    // Pre-instantiate every guest against the now fully-linked
                    // linker and assemble the registry.
                    Ok(Self {
                        registry: Arc::new(compiled.build()?),
                        args,
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
    host_trait_impls: Vec<Path>,
    colist_guard: TokenStream,
    run_fn: TokenStream,
    main_fn: TokenStream,
}

impl TryFrom<&Config> for Expanded {
    type Error = syn::Error;

    fn try_from(input: &Config) -> Result<Self, Self::Error> {
        let mut store_ctx_fields = Vec::new();
        let mut host_trait_impls = Vec::new();
        let mut server_trait_impls = Vec::new();

        // For each backend field, the `StoreCtx` field names that clone from it,
        // keyed by backend field ident in host-declaration order. Emitted as
        // `#[runtime(store = ...)]` on the `Context` backend field so the
        // `Runtime` derive generates the matching `store()` assignment.
        let mut store_targets: Vec<(Ident, Vec<Ident>)> = Vec::new();

        // The backend-less one-shot `wasi:cli` trigger, if listed. It is
        // constructed specially in the command tail (it carries the exit cell),
        // not driven as a bare unit-struct value like every other server.
        let mut cli_type: Option<Path> = None;

        for host in &input.hosts {
            let host_type = &host.type_;
            host_trait_impls.push(host_type.clone());

            // A backend-less host (e.g. `WasiCli`) contributes no `StoreCtx`
            // backend field or host view; it only links (a no-op for `WasiCli`)
            // and drives an export.
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

            if is_wasi_cli(host_type) {
                cli_type = Some(host_type.clone());
            } else {
                server_trait_impls.push(quote! {#host_type});
            }
        }

        // Guardrail: a one-shot command runs to completion and exits, so it
        // cannot share a deployment with a long-lived trigger server (`serve`
        // would never return). Instead of matching host names here — which would
        // miss a future trigger or trip on an aliased import — emit a `const`
        // check over each listed host's `Server::KIND`, so the classification
        // lives in the type system. Capability hosts (`HostKind::Capability`)
        // are fine; the check is a no-op unless a one-shot host is present.
        let colist_guard = if cli_type.is_some() {
            quote! {
                const _: () = omnia::assert_hosts(&[
                    #(<#host_trait_impls as Server<Context>>::KIND,)*
                ]);
            }
        } else {
            quote! {}
        };

        // `Context` backend fields (deduplicated upstream in `Config`). Each
        // carries its `#[runtime(store = ...)]` attributes plus the backend
        // connection plumbing.
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

        // The `run` body and `main` shape differ for a one-shot command (which
        // returns the guest's exit status) versus a long-lived server.
        let run_fn =
            cli_type.as_ref().map_or_else(|| server_run_fn(&server_trait_impls), command_run_fn);
        let main_fn = if !input.gen_main {
            quote! {}
        } else if cli_type.is_some() {
            command_main_fn()
        } else {
            server_main_fn()
        };

        Ok(Self {
            context_fields,
            backend_idents,
            backend_types,
            store_ctx_fields,
            host_trait_impls,
            colist_guard,
            run_fn,
            main_fn,
        })
    }
}

/// The long-lived server `run`: compile, prepare state, then drive every
/// trigger server to completion via `omnia::serve`.
fn server_run_fn(server_trait_impls: &[TokenStream]) -> TokenStream {
    quote! {
        /// Run a guest (single-file shorthand) or a manifest-driven deployment.
        pub async fn run(
            wasm: Option<PathBuf>, config: Option<PathBuf>, args: Vec<String>,
        ) -> Result<()> {
            let compiled = omnia::RegistryBuilder::new()
                .wasm(wasm)
                .config(config)
                .compile::<StoreCtx>()
                .await
                .context("building runtime")?;
            let run_state = Context::new(compiled, Arc::new(args))
                .await
                .context("preparing runtime state")?;

            // Every server runs against the same `run_state`, so they share one
            // registry and therefore one `Engine` (and its pooling allocator's
            // pool). `omnia::serve` drives epoch interruption, pool-metric
            // sampling, and the host-mediated link serve side around them.
            let servers: Vec<BoxFuture<'_, Result<()>>> =
                vec![#(Box::pin(#server_trait_impls.run(&run_state)),)*];
            omnia::serve(&run_state, servers).await.context("starting runtime services")
        }
    }
}

/// The one-shot command `run`: identical compile + state + `serve` as the
/// server, but it drives only the `wasi:cli` trigger and returns the guest's
/// exit status, read from a cell the trigger fills.
fn command_run_fn(cli_type: &Path) -> TokenStream {
    quote! {
        /// Drive the sole `wasi:cli/run` guest once and return its exit status.
        pub async fn run(
            wasm: Option<PathBuf>, config: Option<PathBuf>, args: Vec<String>,
        ) -> Result<omnia::ExitStatus> {
            // argv[0] is conventionally the program name (the `wasmtime` CLI
            // does the same), so derive it from the wasm file and prepend it to
            // the user's arguments before they reach the guest.
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
            let exit = Arc::new(::std::sync::OnceLock::new());
            let run_state = Context::new(compiled, Arc::new(argv))
                .await
                .context("preparing runtime state")?;

            // The command is the only driven server; any co-listed capability
            // host is linked in `Context::new` but its `Server::run` is a no-op.
            // Bind the trigger to a local: unlike the unit-struct servers (which
            // const-promote), `WasiCli` carries the exit cell, so the future
            // `run` returns must borrow a named value that outlives `serve`.
            let cli = #cli_type::new(exit.clone());
            let servers: Vec<BoxFuture<'_, Result<()>>> = vec![Box::pin(cli.run(&run_state))];
            omnia::serve(&run_state, servers).await.context("starting runtime services")?;

            // `serve` returns once the one-shot completes; the cell holds the
            // guest's status (success if it never ran).
            Ok(exit.get().copied().unwrap_or(omnia::ExitStatus::SUCCESS))
        }
    }
}

/// The server entrypoint: parse the CLI and run to completion, propagating any
/// error as a process failure.
fn server_main_fn() -> TokenStream {
    quote! {
        use omnia::tokio;

        #[tokio::main]
        async fn main() -> anyhow::Result<()> {
            use omnia::Parser;
            match omnia::Cli::parse().command {
                omnia::Command::Run { wasm, config, args } => {
                    runtime::run(wasm, config, args).await
                }
                _ => unreachable!(),
            }
        }
    }
}

/// The command entrypoint: parse the CLI, run the one-shot, and exit with the
/// guest's status (or a failure on a host error).
fn command_main_fn() -> TokenStream {
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

/// The last path segment of a host type (e.g. `WasiHttp` for
/// `omnia_wasi_http::WasiHttp`), used to recognise known hosts by name.
fn last_segment(path: &Path) -> String {
    path.segments.last().map_or_else(String::new, |segment| segment.ident.to_string())
}

/// Whether a host is the one-shot `wasi:cli` trigger.
fn is_wasi_cli(path: &Path) -> bool {
    last_segment(path) == "WasiCli"
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
