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
    /// WASI host trait types declared in `hosts`, in declaration order.
    fn host_trait_impls(&self) -> Vec<Path> {
        self.hosts.iter().map(|host| host.type_.clone()).collect()
    }

    /// WASI host linking in `Context::new`. In command mode, long-lived trigger
    /// hosts ([`Server::IS_SERVER`]) are skipped so only capability hosts link.
    pub(crate) fn link_hosts(&self) -> TokenStream {
        let host_trait_impls = self.host_trait_impls();
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
    pub(crate) fn servers(&self) -> TokenStream {
        let host_trait_impls = self.host_trait_impls();
        if self.command {
            quote! { vec![] }
        } else {
            quote! { vec![#(Box::pin(#host_trait_impls.run(&run_state)),)*] }
        }
    }

    /// Concurrent backend connection for `Context::new`.
    pub(crate) fn connect_backends(&self) -> TokenStream {
        if self.backends.is_empty() {
            return quote! {};
        }

        let backend_idents: Vec<Ident> = self.backends.iter().map(field_ident).collect();
        let backend_types = &self.backends;
        quote! {
            let (#(#backend_idents,)*) = tokio::try_join!(
                #(<#backend_types as Backend>::connect(),)*
            )?;
        }
    }

    /// Structural codegen derived from the parsed host/backend wiring.
    pub fn expanded(&self) -> Expanded {
        let mut store_ctx_fields = Vec::new();
        let mut store_targets: BTreeMap<String, Vec<Ident>> = BTreeMap::new();

        for host in &self.hosts {
            let host_type = &host.type_;

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

        let mut context_fields = Vec::new();
        let mut backend_idents = Vec::new();

        for backend in &self.backends {
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
        }

        Expanded {
            context_fields,
            backend_idents,
            store_ctx_fields,
        }
    }
}

/// Codegen fragments derived from a [`Config`]'s host/backend wiring.
pub struct Expanded {
    pub context_fields: Vec<TokenStream>,
    pub backend_idents: Vec<Ident>,
    pub store_ctx_fields: Vec<TokenStream>,
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
    syn::custom_keyword!(main);
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
