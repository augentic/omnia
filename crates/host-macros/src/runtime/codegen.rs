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

        let (backends_ty, backends_def) = emit_backends(host_entries);

        let manifest = emit_manifest(config);
        let main_options = emit_main_options(config, manifest.is_some());

        Self {
            mode: config.mode,
            host_types,
            backends_ty,
            backends_def,
            main_options,
            manifest,
        }
    }
}

fn emit_backends(host_entries: &[HostEntry]) -> (TokenStream, TokenStream) {
    // order-preserving dedup: a backend shared by non-adjacent hosts must not
    // emit two fields; parse validation made their options agree
    let mut backends: Vec<(&Path, Option<&Expr>)> = Vec::new();
    let mut slots: Vec<usize> = Vec::with_capacity(host_entries.len());
    for entry in host_entries {
        let key = path_key(&entry.backend);
        let found = backends.iter().position(|(backend, _)| path_key(backend) == key);
        slots.push(found.unwrap_or_else(|| {
            backends.push((&entry.backend, entry.options.as_ref()));
            backends.len() - 1
        }));
    }

    // positional, so two backends sharing a type name never share a field
    let idents: Vec<Ident> =
        (0..backends.len()).map(|slot| format_ident!("backend_{slot}")).collect();
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
                || quote! { <#ty as Connect>::connect() },
                |options| quote! { <#ty as Connect>::connect_with(#options) },
            )
        })
        .collect();

    let host_impls: Vec<TokenStream> = host_entries
        .iter()
        .zip(&slots)
        .map(|(entry, slot)| host_impl(&entry.host, &idents[*slot]))
        .collect();

    (
        quote! { Backends },
        quote! {
            use omnia::Connect;

            #[derive(Clone)]
            struct Backends {#(
                #idents: #types,
            )*}

            impl omnia::ConnectAll for Backends {
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

// `&mut self.field` coerces to every carrier's `HostCtx::Borrow`, so
// third-party hosts and re-exports work with no name surgery
fn host_impl(host: &Path, field: &Ident) -> TokenStream {
    quote! {
        impl omnia::Provides<#host> for Backends {
            fn borrow(&mut self) -> <#host as omnia::HostCtx>::Borrow<'_> {
                &mut self.#field
            }
        }
    }
}

// `Path` for a `manifest:` expression, `Inline` for the inline keys
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

fn emit_manifest_builder(manifest: &ManifestSpec) -> TokenStream {
    // the registries expression is the configuration's contents, compiled in
    let plugins = manifest.plugins.as_ref().map(|plugins| {
        let store = plugins.store.as_ref().map(|path| {
            quote! {
                .store(::std::path::PathBuf::from(#path))
            }
        });
        let registries = plugins.registries.as_ref().map(|config| {
            quote! {
                .registries(omnia::RegistryConfig::contents(#config))
            }
        });
        quote! {
            #store
            #registries
        }
    });

    // unless the guest names itself, an embedded component is named by its
    // path's stem and a package by its reference without the version
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
        // validated at parse; lands as the bytes it decodes to
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
            #plugins
            #(#guests)*
            #(#mounts)*
    }
}

// no call is emitted for a key the invocation omits
fn emit_main_options(config: &Config, has_manifest: bool) -> TokenStream {
    let mode = config.mode.tokens();
    let manifest = has_manifest.then(|| quote! { .manifest(manifest()) });

    quote! {
        omnia::MainOptions::new(#mode)
            .program_name(env!("CARGO_PKG_NAME"))
            #manifest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(row: &str) -> HostEntry {
        syn::parse_str(row).expect("valid host row")
    }

    #[test]
    fn positional_fields() {
        let entries = [entry("WasiKeyValue: keyvalue::Client"), entry("WasiModel: model::Client")];
        let (ty, def) = emit_backends(&entries);
        assert_eq!(ty.to_string(), "Backends");
        let def = def.to_string();
        assert!(def.contains("backend_0 : keyvalue :: Client"), "{def}");
        assert!(def.contains("backend_1 : model :: Client"), "{def}");
    }

    #[test]
    fn empty_host_entries() {
        let (ty, def) = emit_backends(&[]);
        assert_eq!(ty.to_string(), "()");
        assert!(def.is_empty());
    }
}
