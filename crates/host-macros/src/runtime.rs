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
    fn codegen(&self) -> Codegen {
        let host_trait_impls =
            self.hosts.iter().map(|host| host.type_.clone()).collect::<Vec<Path>>();
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

    /// Server futures passed to [`omnia::run`]: empty in command mode.
    fn servers(&self, host_trait_impls: &[Path]) -> TokenStream {
        if self.command {
            return quote! { |_| vec![] };
        }

        quote! {
            |ctx| {
                let mut servers: Vec<omnia::futures::future::BoxFuture<'_, Result<()>>> = vec![];
                #(
                    if <#host_trait_impls as Server<Context>>::IS_SERVER {
                        servers.push(
                            Box::pin(#host_trait_impls.run(ctx))
                                as omnia::futures::future::BoxFuture<'_, Result<()>>,
                        );
                    }
                )*
                servers
            }
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

// All token fragments needed to expand a deployment runtime.
struct Codegen {
    command: bool,
    context_fields: Vec<TokenStream>,
    backend_idents: Vec<Ident>,
    store_ctx_fields: Vec<TokenStream>,
    link_hosts: TokenStream,
    connect_backends: TokenStream,
    servers: TokenStream,
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
        // server_sync_assertions,
    } = config.codegen();

    quote! {
        mod runtime {
            use std::sync::Arc;

            use anyhow::Result;
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
                #[runtime(args)]
                args: Arc<Vec<String>>,
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
            #[derive(StoreContext)]
            pub struct StoreCtx {
                #[base]
                pub base: StoreBase,
                #(#store_ctx_fields,)*
            }

            #[tokio::main]
            pub async fn main() -> ::std::process::ExitCode {
                omnia::main(#command, Context::new, #servers).await
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
/// Used at macro-expansion time to omit trigger [`StoreCtx`] fields in command
/// mode (struct fields cannot be filtered via `IS_SERVER` in generated code).
/// Generated code asserts this list stays in sync with each host's
/// `Server::IS_SERVER` value.
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
