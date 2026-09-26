//! # Codegen for the runtime macro.
//!
//! Generates the token stream fragments required to expand the runtime macro.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Expr, Ident, Path};

use crate::runtime::parse::{Config, GuestSource, HostEntry, ManifestSpec, Mode};

// Token fragments needed to expand the runtime macro.
pub struct Codegen {
    pub mode: Mode,
    pub host_types: Vec<Path>,
    /// The `Provides` keys the generic `Hooks` impl bounds its bundle on,
    /// one per `hosts:` row.
    pub ctx_keys: Vec<TokenStream>,
    pub backends_ty: TokenStream,
    pub backends_def: TokenStream,
    pub main_options: TokenStream,
    /// The compiled-in `omnia::ManifestSource`; absent when the invocation
    /// declares neither `manifest:` nor inline manifest keys.
    pub manifest: Option<TokenStream>,
}

impl From<&Config> for Codegen {
    fn from(config: &Config) -> Self {
        let host_entries = &config.host_entries;
        let host_types: Vec<Path> = host_entries.iter().map(|entry| entry.host.clone()).collect();
        let ctx_keys = host_types.iter().map(ctx_key).collect();

        let (backends_ty, backends_def) = emit_backends(host_entries);

        let manifest = emit_manifest(config);
        let main_options = emit_main_options(config, manifest.is_some());

        Self {
            mode: config.mode,
            host_types,
            ctx_keys,
            backends_ty,
            backends_def,
            main_options,
            manifest,
        }
    }
}

/// Emit the `omnia::MainOptions` method chain passed to `omnia::main`: the
/// invoking crate's package name as the program name, a compiled-in
/// manifest riding the generated `manifest()` accessor, and no call for a
/// key the invocation omits.
fn emit_main_options(config: &Config, has_manifest: bool) -> TokenStream {
    let mode = config.mode.tokens();
    let manifest = has_manifest.then(|| quote! { .manifest(manifest()) });

    quote! {
        omnia::MainOptions::new(#mode)
            .program_name(env!("CARGO_PKG_NAME"))
            #manifest
    }
}

/// Emit the `omnia::ManifestSource` for the compiled-in deployment
/// manifest: `Path` for a `manifest:` expression, `Inline` for the inline
/// manifest keys, nothing when neither is declared.
fn emit_manifest(config: &Config) -> Option<TokenStream> {
    if let Some(expr) = &config.manifest_file {
        return Some(quote! {
            omnia::ManifestSource::Path(::std::path::PathBuf::from(#expr))
        });
    }
    if config.manifest.is_empty() {
        return None;
    }

    let builder = emit_manifest_builder(&config.manifest);
    Some(quote! {
        omnia::ManifestSource::Inline(#builder)
    })
}

/// Emit the fluent `omnia::Manifest` builder chain for the inline keys.
fn emit_manifest_builder(manifest: &ManifestSpec) -> TokenStream {
    // The expression is the configuration's contents, compiled in; a
    // manifest file names a path instead.
    let registries = manifest.registries.as_ref().map(|config| {
        quote! {
            .registries(omnia::RegistryConfig::contents(#config))
        }
    });

    // An embedded component is read at build time and named by the path's
    // file stem unless the guest names itself; a package is a reference the
    // runtime fetches at first use, named by the reference without its
    // version unless the guest names itself.
    let guests = manifest.guests.iter().map(|guest| {
        let entry = match &guest.source {
            GuestSource::Embedded { path, name: None } => {
                quote! { omnia::GuestEntry::embedded(#path, include_bytes!(#path)) }
            }
            GuestSource::Embedded {
                path,
                name: Some(name),
            } => {
                quote! { omnia::GuestEntry::new(#name, include_bytes!(#path)) }
            }
            GuestSource::Package {
                reference,
                name: None,
            } => {
                quote! { omnia::GuestEntry::package(#reference) }
            }
            GuestSource::Package {
                reference,
                name: Some(name),
            } => {
                quote! { omnia::GuestEntry::new(#name, omnia::SourceSpec::package(#reference)) }
            }
        };
        let http = &guest.routes.http;
        let messaging = &guest.routes.messaging;
        let websocket = &guest.routes.websocket;
        let command = guest.command.then(|| quote! { .command() });
        // The pin was validated at parse; it lands as the bytes it decodes to.
        let digest = guest.digest.as_ref().map(|bytes| {
            let bytes = bytes.iter();
            quote! { .digest(omnia::Digest::from([#(#bytes),*])) }
        });
        quote! {
            .guest(
                #entry
                    #(.route_http(#http))*
                    #(.route_messaging(#messaging))*
                    #(.route_websocket(#websocket))*
                    #command
                    #digest
            )
        }
    });

    let mounts = manifest.mounts.iter().map(|mount| {
        let name = &mount.name;
        let path = &mount.path;
        let writable =
            mount.writable.as_ref().map_or_else(|| quote!(false), ToTokens::to_token_stream);
        quote! {
            .mounts([omnia::Mount {
                name: ::std::string::String::from(#name),
                path: ::std::path::PathBuf::from(#path),
                writable: #writable,
            }])
        }
    });

    quote! {
        omnia::Manifest::new()
            #registries
            #(#guests)*
            #(#mounts)*
    }
}

fn emit_backends(host_entries: &[HostEntry]) -> (TokenStream, TokenStream) {
    // Order-preserving dedup: `Vec::dedup_by` only removes *consecutive*
    // duplicates, so a backend shared by non-adjacent hosts would emit two
    // identically named struct fields. Parse validation guarantees rows
    // sharing a backend agree on connect options, so the first row's
    // options stand for the shared connection.
    let rows = host_entries.iter().map(|entry| (&entry.backend, entry.options.as_ref()));
    let mut seen = std::collections::HashSet::new();
    let backends: Vec<(&Path, Option<&Expr>)> =
        rows.filter(|(backend, _)| seen.insert(path_key(backend))).collect();

    let idents: Vec<Ident> = backends.iter().map(|(backend, _)| field_ident(backend)).collect();
    let types: Vec<&Path> = backends.iter().map(|(backend, _)| *backend).collect();

    if idents.is_empty() {
        return (quote! { () }, quote! {});
    }

    // `Host: Backend(options)` compiles the options in; a bare row connects
    // from the environment.
    let connects: Vec<TokenStream> = backends
        .iter()
        .map(|(ty, options)| {
            options.map_or_else(
                || quote! { <#ty as Backend>::connect() },
                |options| quote! { <#ty as Backend>::connect_with(#options) },
            )
        })
        .collect();

    let host_impls: Vec<TokenStream> = host_entries
        .iter()
        .map(|entry| host_impl(&entry.host, &field_ident(&entry.backend)))
        .collect();

    (
        quote! { Backends },
        quote! {
            use omnia::Backend;

            #[derive(Clone)]
            struct Backends {#(
                #idents: #types,
            )*}

            impl omnia::Backends for Backends {
                async fn connect() -> Result<Self> {
                    let (#(#idents,)*) = tokio::try_join!(
                        #(#connects,)*
                    )?;
                    Ok(Self { #(#idents,)* })
                }
            }

            #(#host_impls)*
        },
    )
}

fn path_key(path: &Path) -> String {
    path.to_token_stream().to_string()
}

/// One uniform bundle-accessor impl per `hosts:` row. The borrow shape rides
/// the carrier's `HostCtx::Borrow` — `&mut self.field` coerces to every
/// carrier's borrow (`&mut dyn Ctx`, `&dyn Ctx`, or `&mut dyn HttpBorrow`) —
/// so third-party hosts and re-exports work with no name surgery.
fn host_impl(host: &Path, field: &Ident) -> TokenStream {
    let ctx = ctx_key(host);
    quote! {
        impl omnia::Provides<#ctx> for Backends {
            fn borrow(&mut self) -> <#ctx as omnia::HostCtx>::Borrow<'_> {
                &mut self.#field
            }
        }
    }
}

/// The `Provides` key for a `hosts:` row: the host type itself, save one
/// special case. `wasi:http`'s linker-facing view trait (`WasiHttpView`) is
/// foreign — owned by `wasmtime-wasi-http` — so its `StoreCtx` blanket lives
/// in omnia core against the core-owned `HttpCtx` carrier, and the http row's
/// accessor must be keyed to that carrier. (Keying by an associated type on
/// the host — `<#host as HostBinding>::Ctx` — is not an option: coherence
/// does not normalize projections in impl headers across crates, so two such
/// impls are rejected as overlapping.) An aliased `WasiHttp` row that dodges
/// this match fails loudly at compile time: linking requires `WasiHttpView`,
/// whose blanket bound `Provides<HttpCtx>` is then unsatisfied.
fn ctx_key(host: &Path) -> TokenStream {
    let is_http = host.segments.last().is_some_and(|segment| segment.ident == "WasiHttp");
    if is_http { quote!(omnia::HttpCtx) } else { quote!(#host) }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> Path {
        syn::parse_str(name).expect("valid path")
    }

    #[test]
    fn derives_field_ident() {
        assert_eq!(field_ident(&path("HttpDefault")).to_string(), "http_default");
        assert_eq!(field_ident(&path("KeyValueDefault")).to_string(), "key_value_default");
    }

    #[test]
    fn empty_host_entries() {
        let (ty, def) = emit_backends(&[]);
        assert_eq!(ty.to_string(), "()");
        assert!(def.is_empty());
    }
}
