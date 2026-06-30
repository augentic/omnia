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

        let field = parse::field_ident(backend_type);
        accessor_impls.push(accessor_impl(&host.type_, &field));
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

// The accessor-impl shape a host needs on the generated `Backends` bundle.
enum AccessorKind {
    Standard,
    Http,
    Config,
}

impl AccessorKind {
    fn from_host(host: &Path) -> Self {
        match host.segments.last().map(|segment| segment.ident.to_string()).as_deref() {
            // `wasi:http`'s view trait is foreign, so its accessor returns a
            // `WasiHttpCtxView` threaded with the store table rather than a
            // borrowed context.
            Some("WasiHttp") => Self::Http,
            // `wasi:config` reads its context through a shared borrow.
            Some("WasiConfig") => Self::Config,
            _ => Self::Standard,
        }
    }
}

// Emit the `HasXxx for Backends` impl wiring a host's connected backend field.
fn accessor_impl(host: &Path, field: &Ident) -> TokenStream {
    match AccessorKind::from_host(host) {
        AccessorKind::Http => quote! {
            impl omnia::HasHttp for Backends {
                fn http_view<'a>(
                    &'a mut self,
                    table: &'a mut omnia::wasmtime_wasi::ResourceTable,
                ) -> omnia_wasi_http::WasiHttpCtxView<'a> {
                    self.#field.as_view(table)
                }
            }
        },
        AccessorKind::Config => {
            let host_crate = parse::wasi_ident(host);
            let has_trait = parse::has_trait(host);
            let ctx_trait = parse::ctx_trait(host);
            let ctx_method = parse::ctx_method(host);
            quote! {
                impl #host_crate::#has_trait for Backends {
                    fn #ctx_method(&self) -> &dyn #host_crate::#ctx_trait {
                        &self.#field
                    }
                }
            }
        }
        AccessorKind::Standard => {
            let host_crate = parse::wasi_ident(host);
            let has_trait = parse::has_trait(host);
            let ctx_trait = parse::ctx_trait(host);
            let ctx_method = parse::ctx_method(host);
            quote! {
                impl #host_crate::#has_trait for Backends {
                    fn #ctx_method(&mut self) -> &mut dyn #host_crate::#ctx_trait {
                        &mut self.#field
                    }
                }
            }
        }
    }
}
