//! # Codegen for the runtime macro.
//!
//! Generates the token streams fragements required to expand the runtime macro.

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
        let structural = structural(config);

        Self {
            command_assert: command_assert(config.command, &host_trait_impls),
            command: config.command,
            store_ctx_fields: structural.store_ctx_fields,
            bundle_fields: structural.bundle_fields,
            store_assignments: structural.store_assignments,
            backend_idents: structural.backend_idents,
            // `backend_idents` is built positionally from `config.backends`, so the
            // connected backend types are exactly that (deduplicated) list.
            backend_types: config.backends.clone(),
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

fn structural(config: &Config) -> Structural {
    let mut store_ctx_fields = Vec::new();
    let mut store_assignments = Vec::new();

    for host in &config.hosts {
        let Some(backend_type) = &host.backend else {
            continue;
        };

        let host_ident = parse::wasi_ident(&host.type_);
        let field = parse::field_ident(backend_type);
        store_ctx_fields.push(quote! {
            #[wasi(#host_ident)]
            pub #host_ident: #backend_type
        });
        // Clone the backend into this host's view field; a backend that backs
        // several hosts naturally yields one assignment per host.
        store_assignments.push(quote! {
            #host_ident: backends.#field.clone()
        });
    }

    let mut bundle_fields = Vec::new();
    let mut backend_idents = Vec::new();

    for backend in &config.backends {
        let field = parse::field_ident(backend);
        bundle_fields.push(quote! {
            #field: #backend
        });
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
