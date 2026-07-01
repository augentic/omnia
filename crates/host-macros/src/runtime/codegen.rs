//! # Codegen for the runtime macro.
//!
//! Generates the token streams fragements required to expand the runtime macro.

use proc_macro2::TokenStream;
use quote::format_ident;
use quote::{ToTokens, quote};
use syn::{Ident, Path};

use crate::runtime::parse::{Config, HostEntry};

// Token fragments needed to expand the runtime macro.
pub struct Codegen {
    pub command: bool,
    pub host_types: Vec<Path>,
    pub host_impls: Vec<TokenStream>,
    pub backend_idents: Vec<Ident>,
    pub backend_types: Vec<Path>,
}

impl From<&Config> for Codegen {
    fn from(config: &Config) -> Self {
        let host_types =
            config.host_entries.iter().map(|entry| entry.host.clone()).collect::<Vec<Path>>();
        let structural = Structural::from(config);

        Self {
            command: config.command,
            host_types,
            host_impls: structural.host_impls,
            backend_idents: structural.backend_idents,
            backend_types: structural.backend_types,
        }
    }
}

struct Structural {
    host_impls: Vec<TokenStream>,
    backend_idents: Vec<Ident>,
    backend_types: Vec<Path>,
}

impl From<&Config> for Structural {
    fn from(config: &Config) -> Self {
        let mut host_impls = Vec::new();

        for entry in &config.host_entries {
            let backend_type = &entry.backend;
            let field = field_ident(backend_type);
            host_impls.push(host_impl(&entry.host, &field));
        }

        let backend_types = unique_backends(&config.host_entries);
        let backend_idents = backend_types.iter().map(field_ident).collect();

        Self {
            host_impls,
            backend_idents,
            backend_types,
        }
    }
}

fn unique_backends(host_entries: &[HostEntry]) -> Vec<Path> {
    let mut backends: Vec<Path> = host_entries.iter().map(|entry| entry.backend.clone()).collect();
    backends.sort_by_cached_key(to_key);
    backends.dedup_by(|a, b| to_key(a) == to_key(b));
    backends
}

fn to_key(path: &Path) -> String {
    path.to_token_stream().to_string()
}

// The accessor-impl shape a host needs on the generated `Backends` bundle.
enum HostType {
    Standard,
    Http,
    Config,
}

impl HostType {
    fn from_host(host: &Path) -> Self {
        match host.segments.last().map(|segment| segment.ident.to_string()).as_deref() {
            // `wasi:http`'s view trait is foreign, so its accessor returns a
            // `WasiHttpCtxView`
            Some("WasiHttp") => Self::Http,
            // `wasi:config` reads its context through a shared borrow.
            Some("WasiConfig") => Self::Config,
            _ => Self::Standard,
        }
    }
}

// Emit the `HasXxx for Backends` impl wiring a host's connected backend field.
fn host_impl(host: &Path, field: &Ident) -> TokenStream {
    match HostType::from_host(host) {
        HostType::Http => quote! {
            impl omnia::HasHttp for Backends {
                fn http_view<'a>(
                    &'a mut self,
                    table: &'a mut omnia::wasmtime_wasi::ResourceTable,
                ) -> omnia_wasi_http::WasiHttpCtxView<'a> {
                    self.#field.as_view(table)
                }
            }
        },
        HostType::Config => {
            let host_crate = wasi_ident(host);
            let has_trait = has_trait(host);
            let ctx_trait = ctx_trait(host);
            let ctx_method = ctx_method(host);
            quote! {
                impl #host_crate::#has_trait for Backends {
                    fn #ctx_method(&self) -> &dyn #host_crate::#ctx_trait {
                        &self.#field
                    }
                }
            }
        }
        HostType::Standard => {
            let host_crate = wasi_ident(host);
            let has_trait = has_trait(host);
            let ctx_trait = ctx_trait(host);
            let ctx_method = ctx_method(host);
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

// Recover a host type's service stem by stripping the `Wasi` prefix from its
// final path segment (e.g. `WasiJsonDb` -> `JsonDb`).
fn service_stem(path: &Path) -> String {
    let Some(segment) = path.segments.last() else {
        return String::new();
    };

    let name = segment.ident.to_string();
    name.strip_prefix("Wasi").unwrap_or(name.as_str()).to_string()
}

/// Derives the bundle accessor trait ident (e.g. `WasiJsonDb` -> `HasJsonDb`).
fn has_trait(path: &Path) -> Ident {
    format_ident!("Has{}", service_stem(path))
}

// Derive the backend context trait ident (e.g. `WasiJsonDb` -> `WasiJsonDbCtx`).
fn ctx_trait(path: &Path) -> Ident {
    format_ident!("Wasi{}Ctx", service_stem(path))
}

// Derive the bundle accessor method ident (e.g. `WasiJsonDb` -> `jsondb_ctx`).
fn ctx_method(path: &Path) -> Ident {
    format_ident!("{}_ctx", service_stem(path).to_lowercase())
}

// Derive a `snake_case` field name from a backend type's final path segment
// (e.g. `HttpDefault` -> `http_default`).
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

// Derive a host crate's module ident from a host type's final path segment
// (e.g. `WasiHttp` -> `omnia_wasi_http`), naming the host crate whose bundle
// accessor trait the generated `Backends` impl satisfies.
fn wasi_ident(path: &Path) -> Ident {
    let Some(segment) = path.segments.last() else {
        return format_ident!("wasi");
    };

    let name = segment.ident.to_string().replace("Wasi", "omnia_wasi_").to_lowercase();
    format_ident!("{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(name: &str) -> Path {
        syn::parse_str(name).expect("valid host path")
    }

    #[test]
    fn derives_accessor_names() {
        for (input, has, ctx, method) in [
            ("WasiJsonDb", "HasJsonDb", "WasiJsonDbCtx", "jsondb_ctx"),
            ("WasiWebSocket", "HasWebSocket", "WasiWebSocketCtx", "websocket_ctx"),
            ("WasiKeyValue", "HasKeyValue", "WasiKeyValueCtx", "keyvalue_ctx"),
        ] {
            let path = host(input);
            assert_eq!(has_trait(&path).to_string(), has);
            assert_eq!(ctx_trait(&path).to_string(), ctx);
            assert_eq!(ctx_method(&path).to_string(), method);
        }
    }

    #[test]
    fn derives_crate_ident() {
        assert_eq!(wasi_ident(&host("WasiJsonDb")).to_string(), "omnia_wasi_jsondb");
        assert_eq!(wasi_ident(&host("WasiWebSocket")).to_string(), "omnia_wasi_websocket");
    }
}
