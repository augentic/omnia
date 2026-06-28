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
    pub store_ctx_fields: Vec<TokenStream>,
    pub bundle_fields: Vec<TokenStream>,
    pub store_assignments: Vec<TokenStream>,
    pub backend_idents: Vec<Ident>,
    pub backend_types: Vec<Path>,
    pub host_trait_impls: Vec<Path>,
}

impl From<&Config> for Codegen {
    fn from(config: &Config) -> Self {
        let host_trait_impls =
            config.hosts.iter().map(|host| host.type_.clone()).collect::<Vec<Path>>();
        let structural = structural(config, &host_trait_impls);

        let backend_types = structural
            .backend_idents
            .iter()
            .map(|ident| {
                config
                    .backends
                    .iter()
                    .find(|backend| parse::field_ident(backend) == *ident)
                    .expect("wired backend must be declared in `hosts`")
            })
            .cloned()
            .collect::<Vec<Path>>();

        Self {
            command_assert: command_assert(config.command, &host_trait_impls),
            command: config.command,
            store_ctx_fields: structural.store_ctx_fields,
            bundle_fields: structural.bundle_fields,
            store_assignments: structural.store_assignments,
            backend_idents: structural.backend_idents,
            backend_types,
            host_trait_impls,
        }
    }
}

struct Structural {
    store_ctx_fields: Vec<TokenStream>,
    bundle_fields: Vec<TokenStream>,
    store_assignments: Vec<TokenStream>,
    backend_idents: Vec<Ident>,
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

    let mut bundle_fields = Vec::new();
    let mut store_assignments = Vec::new();
    let mut backend_idents = Vec::new();

    for backend in &config.backends {
        let field = parse::field_ident(backend);
        bundle_fields.push(quote! {
            #field: #backend
        });

        // Clone this backend into each host-view field it backs (one backend may
        // back several hosts).
        if let Some(targets) = store_targets.get(&field.to_string()) {
            for target in targets {
                store_assignments.push(quote! {
                    #target: backends.#field.clone()
                });
            }
        }

        backend_idents.push(field);
    }

    Structural {
        store_ctx_fields,
        bundle_fields,
        store_assignments,
        backend_idents,
    }
}

fn command_assert(command: bool, host_trait_impls: &[Path]) -> TokenStream {
    if !command {
        return quote! {};
    }

    quote! {
        const _: () = omnia::assert_hosts(&[
            #( <#host_trait_impls as Server<Ctx>>::IS_SERVER, )*
        ]);
    }
}
