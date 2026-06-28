//! # Runtime macro configuration and expansion
//!
//! Parses `runtime!({ ... })` and expands it into a complete runtime module.

use std::collections::BTreeMap;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitBool, Path, Result, Token};

/// Configuration for the runtime macro.
///
/// Parses input in the form of 'host:backend' pairs. For example:
/// ```ignore
/// {
///     WasiHttp: HttpDefault,
///     WasiOtel: DefaultOtel,
///     ...
/// }
/// ```
pub struct Config {
    pub command: bool,
    pub hosts: Vec<Host>,
    pub backends: Vec<Path>,
}

impl Config {
    /// All token fragments needed to expand a deployment runtime.
    pub fn codegen(&self) -> Codegen {
        let host_trait_impls = self.host_trait_impls();
        let structural = self.structural(&host_trait_impls);

        Codegen {
            command: self.command,
            context_fields: structural.context_fields,
            backend_idents: structural.backend_idents.clone(),
            store_ctx_fields: structural.store_ctx_fields,
            link_hosts: self.link_hosts(&host_trait_impls),
            connect_backends: self.connect_backends(&structural.backend_idents),
            servers: self.servers(&host_trait_impls),
        }
    }

    /// WASI host trait types declared in `hosts`, in declaration order.
    fn host_trait_impls(&self) -> Vec<Path> {
        self.hosts.iter().map(|host| host.type_.clone()).collect()
    }

    /// Structural `Context` / `StoreCtx` field fragments.
    fn structural(&self, host_trait_impls: &[Path]) -> Structural {
        let mut store_ctx_fields = Vec::new();
        let mut store_targets: BTreeMap<String, Vec<Ident>> = BTreeMap::new();

        for (host, host_type) in self.hosts.iter().zip(host_trait_impls) {
            // A backend-less host contributes no `StoreCtx` backend field or
            // host view; it only links its interface.
            let Some(backend_type) = &host.backend else {
                continue;
            };

            // In command mode, long-lived triggers are not linked or wired.
            if self.command && is_trigger_host(host_type) {
                continue;
            }

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

        let mut context_fields = Vec::new();
        let mut backend_idents = Vec::new();

        for backend in &self.backends {
            let field = field_ident(backend);
            let Some(targets) = store_targets.get(&field.to_string()) else {
                if self.command {
                    continue;
                }
                context_fields.push(quote! {
                    pub #field: #backend
                });
                backend_idents.push(field);
                continue;
            };

            let store_attrs: Vec<TokenStream> =
                targets.iter().map(|target| quote! { #[runtime(store = #target)] }).collect();

            context_fields.push(quote! {
                #(#store_attrs)*
                pub #field: #backend
            });
            backend_idents.push(field);
        }

        Structural {
            context_fields,
            backend_idents,
            store_ctx_fields,
        }
    }

    /// WASI host linking in `Context::new`. In command mode, long-lived trigger
    /// hosts ([`Server::IS_SERVER`]) are skipped so only capability hosts link.
    fn link_hosts(&self, host_trait_impls: &[Path]) -> TokenStream {
        if self.command {
            quote! {
                #(
                    if !<#host_trait_impls as Server<Context>>::IS_SERVER {
                        compiled.host::<#host_trait_impls>()?;
                    }
                )*
            }
        } else {
            quote! {
                #(compiled.host::<#host_trait_impls>()?;)*
            }
        }
    }

    /// Server futures passed to [`omnia::drive`]: empty in command mode.
    fn servers(&self, host_trait_impls: &[Path]) -> TokenStream {
        if self.command {
            return quote! { vec![] };
        }

        let server_hosts: Vec<&Path> =
            host_trait_impls.iter().filter(|host| is_trigger_host(host)).collect();

        quote! {
            vec![#(Box::pin(#server_hosts.run(&run_state)),)*]
        }
    }

    /// Concurrent backend connection for `Context::new`.
    fn connect_backends(&self, backend_idents: &[Ident]) -> TokenStream {
        if backend_idents.is_empty() {
            return quote! {};
        }

        let backend_types: Vec<&Path> = backend_idents
            .iter()
            .map(|ident| {
                self.backends
                    .iter()
                    .find(|backend| field_ident(backend) == *ident)
                    .expect("wired backend must be declared in `hosts`")
            })
            .collect();

        quote! {
            let (#(#backend_idents,)*) = tokio::try_join!(
                #(<#backend_types as Backend>::connect(),)*
            )?;
        }
    }
}

struct Structural {
    context_fields: Vec<TokenStream>,
    backend_idents: Vec<Ident>,
    store_ctx_fields: Vec<TokenStream>,
}

/// All token fragments needed to expand a deployment runtime.
pub struct Codegen {
    pub command: bool,
    pub context_fields: Vec<TokenStream>,
    pub backend_idents: Vec<Ident>,
    pub store_ctx_fields: Vec<TokenStream>,
    pub link_hosts: TokenStream,
    pub connect_backends: TokenStream,
    pub servers: TokenStream,
}

/// Generate the runtime module from a parsed [`Config`].
pub fn expand(config: &Config) -> TokenStream {
    let Codegen {
        command,
        context_fields,
        backend_idents,
        store_ctx_fields,
        link_hosts,
        connect_backends,
        servers,
    } = config.codegen();

    quote! {
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

            // Runtime state holding the guest registry and backend connections.
            #[derive(Clone, Runtime)]
            #[runtime(store = StoreCtx)]
            struct Context {
                #[runtime(registry)]
                registry: Arc<Registry<StoreCtx>>,
                // Guest argv threaded into every store (empty for servers; in
                // command mode `args[0]` is the program name).
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
                async fn new(mut compiled: Compiled<StoreCtx>) -> Result<Self> {
                    let args = Arc::new(compiled.args().to_vec());

                    // link enabled WASI components
                    #link_hosts

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
                let compiled = omnia::RegistryBuilder::new()
                    .wasm(wasm)
                    .config(config)
                    .args(args)
                    .command(#command)
                    .compile::<StoreCtx>()
                    .await
                    .context("building runtime")?;
                let run_state = Context::new(compiled)
                    .await
                    .context("preparing runtime state")?;

                omnia::run(&run_state, #command, #servers)
                    .await
                    .context("running deployment")
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
    }
}

impl Parse for Config {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut command = false;
        let mut hosts = Hosts(Vec::new());

        let settings;
        syn::braced!(settings in input);
        let settings = Punctuated::<Opt, Token![,]>::parse_terminated(&settings)?;

        for setting in settings.into_pairs() {
            match setting.into_value() {
                Opt::Command(c) => command = c,
                Opt::Host(h) => hosts = h,
            }
        }

        // deduplicate backends (skipping backend-less hosts)
        let mut backends: Vec<Path> = vec![];
        for host in &hosts.0 {
            let Some(backend) = &host.backend else {
                continue;
            };
            if backends.iter().any(|b| b.get_ident() == backend.get_ident()) {
                continue;
            }
            backends.push(backend.clone());
        }

        Ok(Self {
            command,
            hosts: hosts.0,
            backends,
        })
    }
}

mod kw {
    syn::custom_keyword!(command);
    syn::custom_keyword!(hosts);
}

#[allow(clippy::large_enum_variant)]
enum Opt {
    Command(bool),
    Host(Hosts),
}

impl Parse for Opt {
    fn parse(input: ParseStream) -> Result<Self> {
        let l = input.lookahead1();
        if l.peek(kw::command) {
            input.parse::<kw::command>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Command(input.parse::<LitBool>()?.value))
        } else if l.peek(kw::hosts) {
            input.parse::<kw::hosts>()?;
            input.parse::<Token![:]>()?;
            let list;
            syn::braced!(list in input);
            Ok(Self::Host(list.parse()?))
        } else {
            Err(l.error())
        }
    }
}

pub struct Hosts(Vec<Host>);

impl Parse for Hosts {
    fn parse(input: ParseStream) -> Result<Self> {
        let hosts = Punctuated::<Host, Token![,]>::parse_terminated(input)?;
        Ok(Self(hosts.into_iter().collect()))
    }
}

/// Information about a WASI host and its configuration.
///
/// `backend` is optional: a host may be declared bare (no `: Backend`) when the
/// backend is selected elsewhere — e.g. the planned deploy-time `mode: dynamic`,
/// where `hosts` lists interfaces only.
pub struct Host {
    pub type_: Path,
    pub backend: Option<Path>,
}

impl Parse for Host {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let type_ = input.parse::<Path>()?;
        // `Host: Backend` (a backend-backed host) or bare `Host` (backend-less).
        let backend = if input.peek(Token![:]) {
            input.parse::<Token![:]>()?;
            Some(input.parse::<Path>()?)
        } else {
            None
        };
        Ok(Self { type_, backend })
    }
}

/// Whether `path` names a long-lived trigger host ([`Server::IS_SERVER`]).
///
/// Kept in sync with the host crates that set `IS_SERVER` to `true`. Used only
/// to omit trigger wiring in command mode at macro-expansion time.
fn is_trigger_host(path: &Path) -> bool {
    path.segments.last().is_some_and(|segment| {
        matches!(segment.ident.to_string().as_str(), "WasiHttp" | "WasiMessaging" | "WasiWebSocket")
    })
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
