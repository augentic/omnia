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
    pub bundle_fields: Vec<TokenStream>,
    pub accessor_impls: Vec<TokenStream>,
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
            command: config.command,
            bundle_fields: structural.bundle_fields,
            accessor_impls: structural.accessor_impls,
            backend_idents: structural.backend_idents,
            backend_types: config.backends.clone(),
            host_trait_impls,
        }
    }
}

struct Structural {
    bundle_fields: Vec<TokenStream>,
    accessor_impls: Vec<TokenStream>,
    backend_idents: Vec<Ident>,
}

fn structural(config: &Config) -> Structural {
    let mut accessor_impls = Vec::new();

    for host in &config.hosts {
        let Some(backend_type) = &host.backend else {
            continue;
        };

        let host_crate = parse::wasi_ident(&host.type_);
        let field = parse::field_ident(backend_type);
        accessor_impls.push(quote! {
            #host_crate::omnia_wasi_view!(Backends, #field);
        });
    }

    let mut bundle_fields = Vec::new();
    let mut backend_idents = Vec::new();

    for backend in &config.backends {
        let field = parse::field_ident(backend);
        bundle_fields.push(quote! { #field: #backend });
        backend_idents.push(field);
    }

    Structural {
        bundle_fields,
        accessor_impls,
        backend_idents,
    }
}
