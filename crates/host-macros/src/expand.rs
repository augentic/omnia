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
    } = Expanded::try_from(config)?;

    // The deployment entrypoint and its crate-root export. With `main: true` the
    // entrypoint is a `#[tokio::main]` re-exported from this module (so it can
    // name `Context`/`StoreCtx`); with `main: false` it is the `pub async fn run`
    // escape hatch a caller-supplied `main` drives, and the export is empty.
    let (entrypoint, entrypoint_export) =
        entrypoint(config.command, config.gen_main, &host_trait_impls);

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

    // `tokio` is referenced by `try_join!` (backend connection) and by the
    // generated `main`'s `#[tokio::main]`; import it once when either is in play.
    let tokio_import = if config.gen_main || !backend_idents.is_empty() {
        quote! { use omnia::tokio; }
    } else {
        quote! {}
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


            #entrypoint
        }

        #entrypoint_export
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

/// The deployment's core orchestration, shared by the generated `main`
/// (`main: true`) and the `pub async fn run` escape hatch (`main: false`):
/// compile the registry and build runtime state, then either drive the one-shot
/// `wasi:cli` command (returning the guest's exit status) or serve every
/// long-lived trigger to completion.
///
/// Expands to a block that expects `wasm`, `config`, and `args` bindings in
/// scope and evaluates to `Result<omnia::ExitStatus>`.
fn run_body(command: bool, hosts: &[Path]) -> TokenStream {
    // The two deployment shapes share the compile + state-build prologue and
    // differ only in the guest argv and how guests are driven. A command makes
    // the deployment name `argv[0]` (`Compiled::argv`, read via
    // `wasi:cli/environment` like `wasmtime`) and drives the sole `wasi:cli/run`
    // guest once; a server inherits the passed-through args (empty unless
    // forwarded) and serves every trigger to completion.
    let (argv, drive) = if command {
        (
            quote! { compiled.argv(args) },
            quote! { omnia::run_command(&run_state).await.context("running command") },
        )
    } else {
        (
            quote! { args },
            quote! {
                let servers: Vec<BoxFuture<'_, Result<()>>> =
                    vec![#(Box::pin(#hosts.run(&run_state)),)*];
                omnia::serve(&run_state, servers).await.context("starting runtime services")?;
                Ok(omnia::ExitStatus::SUCCESS)
            },
        )
    };

    quote! {
        let compiled = omnia::RegistryBuilder::new()
            .wasm(wasm)
            .config(config)
            .compile::<StoreCtx>()
            .await
            .context("building runtime")?;
        // Bound before `Context::new` consumes `compiled`: the command shape
        // borrows it for `argv`, which must end before the move.
        let argv = #argv;
        let run_state = Context::new(compiled, Arc::new(argv))
            .await
            .context("preparing runtime state")?;
        #drive
    }
}

/// Build the deployment entrypoint and its crate-root export from the shared
/// [`run_body`].
///
/// With `gen_main`, the entrypoint is a `#[tokio::main]` that parses the CLI and
/// maps the run to a process exit code. It lives *inside* the runtime module so
/// it can name `Context`/`StoreCtx` directly, and is surfaced as the binary
/// entrypoint by a `use runtime::main;` re-export at the crate root. Without it,
/// the same orchestration is the `pub async fn run` escape hatch a
/// caller-supplied `main` drives, and the export is empty.
fn entrypoint(command: bool, gen_main: bool, hosts: &[Path]) -> (TokenStream, TokenStream) {
    let body = run_body(command, hosts);

    if gen_main {
        let main = quote! {
            /// Parse the CLI and drive the deployment to a process exit code: the
            /// guest's status for a one-shot `command`, success for a long-lived
            /// server's clean shutdown, or failure on a host error.
            #[tokio::main]
            pub async fn main() -> ::std::process::ExitCode {
                use omnia::Parser;
                match omnia::Cli::parse().command {
                    omnia::Command::Run { wasm, config, args } => {
                        // The annotation pins the orchestration's error type to
                        // `anyhow::Error`; without it `?` is ambiguous when a
                        // linked host (e.g. otel) is also in scope with its own
                        // `From<anyhow::Error>`.
                        let outcome: Result<omnia::ExitStatus> = async move { #body }.await;
                        match outcome {
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
        };
        (main, quote! { use runtime::main; })
    } else {
        let run = quote! {
            /// Run a guest (single-file shorthand) or a manifest-driven deployment.
            pub async fn run(
                wasm: Option<PathBuf>, config: Option<PathBuf>, args: Vec<String>,
            ) -> Result<omnia::ExitStatus> {
                #body
            }
        };
        (run, quote! {})
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
