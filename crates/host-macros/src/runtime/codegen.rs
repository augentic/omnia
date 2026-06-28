//! # Codegen for the runtime macro.
//! 
//! Generates the token streams fragements required to expand the runtime macro.

use std::collections::BTreeMap;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Path};

use crate::runtime::parse::{self, Config};

// All token fragments needed to expand a deployment runtime.
pub struct Codegen {
    pub command: bool,
    pub command_assert: TokenStream,
    pub context_fields: Vec<TokenStream>,
    pub backend_idents: Vec<Ident>,
    pub store_ctx_fields: Vec<TokenStream>,
    pub add_to_linker: TokenStream,
    pub connect_backends: TokenStream,
    pub servers: TokenStream,
}

impl From<&Config> for Codegen {
    fn from(config: &Config) -> Self {
        let host_trait_impls =
            config.hosts.iter().map(|host| host.type_.clone()).collect::<Vec<Path>>();
        let structural = structural(config, &host_trait_impls);

        let add_to_linker = quote! {
            #( compiled.host::<#host_trait_impls, Context>()?; )*
        };

        Self {
            command: config.command,
            command_assert: command_assert(config.command, &host_trait_impls),
            context_fields: structural.context_fields,
            backend_idents: structural.backend_idents.clone(),
            store_ctx_fields: structural.store_ctx_fields,
            add_to_linker,
            connect_backends: connect_backends(&config.backends, &structural.backend_idents),
            servers: servers(&host_trait_impls),
        }
    }
}

struct Structural {
    context_fields: Vec<TokenStream>,
    backend_idents: Vec<Ident>,
    store_ctx_fields: Vec<TokenStream>,
}

fn structural(config: &Config, host_trait_impls: &[Path]) -> Structural {
    let mut store_ctx_fields = Vec::new();
    let mut store_targets: BTreeMap<String, Vec<Ident>> = BTreeMap::new();

    for (host, host_type) in config.hosts.iter().zip(host_trait_impls) {
        let Some(backend_type) = &host.backend else {
            continue;
        };

        let host_ident = parse::wasi_ident(host_type);
        let backend_ident = parse::field_ident(backend_type);
        store_ctx_fields.push(quote! {
            #[wasi(#host_ident)]
            pub #host_ident: #backend_type
        });
        store_targets.entry(backend_ident.to_string()).or_default().push(host_ident);
    }

    let mut context_fields = Vec::new();
    let mut backend_idents = Vec::new();

    for backend in &config.backends {
        let field = parse::field_ident(backend);
        let Some(targets) = store_targets.get(&field.to_string()) else {
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

fn connect_backends(backends: &[Path], backend_idents: &[Ident]) -> TokenStream {
    if backend_idents.is_empty() {
        return quote! {};
    }

    let backend_types: Vec<&Path> = backend_idents
        .iter()
        .map(|ident| {
            backends
                .iter()
                .find(|backend| parse::field_ident(backend) == *ident)
                .expect("wired backend must be declared in `hosts`")
        })
        .collect();

    quote! {
        let (#(#backend_idents,)*) = tokio::try_join!(
            #(<#backend_types as Backend>::connect(),)*
        )?;
    }
}

fn servers(host_trait_impls: &[Path]) -> TokenStream {
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

fn command_assert(command: bool, host_trait_impls: &[Path]) -> TokenStream {
    if !command {
        return quote! {};
    }

    quote! {
        const _: () = omnia::assert_hosts(&[
            #( <#host_trait_impls as Server<Context>>::IS_SERVER, )*
        ]);
    }
}
