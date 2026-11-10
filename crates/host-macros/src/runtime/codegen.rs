//! # Codegen for the runtime macro.
//!
//! Generates the token stream fragments required to expand the runtime macro.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Expr, Ident, Path};

use crate::runtime::parse::{
    BackendEntry, Config, GuestSource, HostEntry, Hosts, ManifestSpec, Mode,
};

// Token fragments needed to expand the runtime macro.
pub struct Codegen {
    pub mode: Mode,
    pub wiring: Wiring,
    pub main_options: TokenStream,
    /// The compiled-in `omnia::ManifestSource`; absent when the invocation
    /// declares neither `manifest:` nor inline manifest keys.
    pub manifest: Option<TokenStream>,
}

impl From<&Config> for Codegen {
    fn from(config: &Config) -> Self {
        let wiring = match &config.hosts {
            Hosts::List(entries) if !entries.is_empty() => Wiring::from_list(entries),
            Hosts::List(_) => Wiring::from_map(&[]),
            Hosts::Map(entries) => Wiring::from_map(entries),
        };

        let manifest = emit_manifest(config);
        let main_options = emit_main_options(config, manifest.is_some());

        Self {
            mode: config.mode,
            wiring,
            main_options,
            manifest,
        }
    }
}

// What the `hosts:` key expands to: the bundle and how `Hooks` wires it.
pub struct Wiring {
    /// The bundle's type, as `main` and `run` name it.
    pub backends_ty: TokenStream,
    /// The bundle's definition and its impls.
    pub backends_def: TokenStream,
    /// What `Hooks` asks of a bundle `B`, one where-predicate each.
    pub bounds: Vec<TokenStream>,
    /// Statements linking every host into `deployment`.
    pub link: TokenStream,
    /// An expression collecting every host's `run` future over `runtime`.
    pub servers: TokenStream,
}

impl Wiring {
    fn from_map(host_entries: &[HostEntry]) -> Self {
        let hosts: Vec<&Path> = host_entries.iter().map(|entry| &entry.host).collect();
        let (backends_ty, backends_def) = emit_backends(host_entries);

        Self {
            backends_ty,
            backends_def,
            bounds: hosts.iter().map(|host| quote! { B: omnia::Provides<#host> }).collect(),
            link: quote! { #(deployment.host::<#hosts>()?;)* },
            // capability hosts resolve at once through `Host::run`'s no-op default
            servers: quote! {
                vec![#(
                    Box::pin(<#hosts as omnia::Host<B>>::run(runtime)),
                )*]
            },
        }
    }

    fn from_list(entries: &[BackendEntry]) -> Self {
        // positional, so two backends sharing a type name never share a name
        let idents: Vec<Ident> =
            (0..entries.len()).map(|slot| format_ident!("backend_{slot}")).collect();
        let aliases: Vec<Ident> =
            (0..entries.len()).map(|slot| format_ident!("Hosts{slot}")).collect();

        // a bare entry serves every host its backend names
        let tuples = entries.iter().map(|entry| {
            let backend = &entry.backend;
            entry.hosts.as_ref().map_or_else(
                || quote! { <#backend as omnia::Backend>::Hosts },
                |hosts| quote! { (#(#hosts,)*) },
            )
        });
        let connects =
            entries.iter().map(|entry| emit_connect(&entry.backend, entry.options.as_ref()));

        Self {
            backends_ty: quote! { Backends },
            backends_def: quote! {
                use omnia::Connect;

                #(type #aliases = #tuples;)*

                #[derive(Clone)]
                struct Backends(omnia::Slots);

                impl omnia::ConnectAll for Backends {
                    async fn connect() -> Result<Self> {
                        let (#(#idents,)*) = tokio::try_join!(
                            #(#connects,)*
                        )?;
                        let mut slots = omnia::Slots::new();
                        #(slots.add_as::<_, #aliases>(&#idents)?;)*
                        Ok(Self(slots))
                    }
                }

                impl<H: omnia::SlotHost> omnia::Provides<H> for Backends {
                    fn borrow(&mut self) -> H::Borrow<'_> {
                        <omnia::Slots as omnia::Provides<H>>::borrow(&mut self.0)
                    }
                }
            },
            bounds: aliases.iter().map(|alias| quote! { #alias: omnia::Link<B> }).collect(),
            link: quote! { #(deployment.hosts::<#aliases>()?;)* },
            servers: quote! {{
                let mut servers = Vec::new();
                #(<#aliases as omnia::Link<B>>::run(runtime, &mut servers);)*
                servers
            }},
        }
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

    let connects: Vec<TokenStream> =
        backends.iter().map(|(ty, options)| emit_connect(ty, *options)).collect();

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

// `Backend(options)` compiles the options in; a bare backend connects from
// the environment
fn emit_connect(backend: &Path, options: Option<&Expr>) -> TokenStream {
    options.map_or_else(
        || quote! { <#backend as Connect>::connect() },
        |options| quote! { <#backend as Connect>::connect_with(#options) },
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
