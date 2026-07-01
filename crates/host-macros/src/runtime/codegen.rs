//! # Codegen for the runtime macro.
//!
//! Generates the token stream fragments required to expand the runtime macro.

use proc_macro2::TokenStream;
use quote::format_ident;
use quote::{ToTokens, quote};
use syn::{Ident, Path};

use crate::runtime::parse::Config;

// Token fragments needed to expand the runtime macro.
pub struct Codegen {
    pub command: bool,
    pub host_types: Vec<Path>,
    pub backends_ty: TokenStream,
    pub backends_def: TokenStream,
}

// One connected backend field on the generated [`Backends`] struct.
struct BackendField {
    ident: Ident,
    ty: Path,
}

impl From<&Config> for Codegen {
    fn from(config: &Config) -> Self {
        let host_types = config.host_entries.iter().map(|entry| entry.host.clone()).collect();
        let backend_fields = config.backend_fields();
        let host_impls: Vec<TokenStream> = config
            .host_entries
            .iter()
            .map(|entry| host_impl(&entry.host, &field_ident(&entry.backend)))
            .collect();
        let (backends_ty, backends_def) = emit_backends(&backend_fields, &host_impls);

        Self {
            command: config.command,
            host_types,
            backends_ty,
            backends_def,
        }
    }
}

fn emit_backends(
    backend_fields: &[BackendField],
    host_impls: &[TokenStream],
) -> (TokenStream, TokenStream) {
    if backend_fields.is_empty() {
        return (quote! { () }, quote! {});
    }

    let idents: Vec<_> = backend_fields.iter().map(|field| &field.ident).collect();
    let types: Vec<_> = backend_fields.iter().map(|field| &field.ty).collect();

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
                        #(<#types as Backend>::connect(),)*
                    )?;
                    Ok(Self { #(#idents,)* })
                }
            }

            #(#host_impls)*
        },
    )
}

impl Config {
    fn backend_fields(&self) -> Vec<BackendField> {
        let mut backends: Vec<Path> =
            self.host_entries.iter().map(|entry| entry.backend.clone()).collect();
        backends.dedup_by(|a, b| path_key(a) == path_key(b));
        backends
            .into_iter()
            .map(|ty| BackendField {
                ident: field_ident(&ty),
                ty,
            })
            .collect()
    }
}

fn path_key(path: &Path) -> String {
    path.to_token_stream().to_string()
}

enum HostType {
    Standard,
    Http,
    Config,
}

impl HostType {
    fn from_host(host: &Path) -> Self {
        match host.segments.last().map(|segment| segment.ident.to_string()).as_deref() {
            Some("WasiHttp") => Self::Http,
            Some("WasiConfig") => Self::Config,
            _ => Self::Standard,
        }
    }
}

#[derive(Copy, Clone)]
enum CtxMutability {
    Shared,
    Exclusive,
}

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
        HostType::Config => ctx_impl(host, field, CtxMutability::Shared),
        HostType::Standard => ctx_impl(host, field, CtxMutability::Exclusive),
    }
}

fn ctx_impl(host: &Path, field: &Ident, mutability: CtxMutability) -> TokenStream {
    let host_crate = wasi_ident(host);
    let has_trait = has_trait(host);
    let ctx_trait = ctx_trait(host);
    let ctx_method = ctx_method(host);

    match mutability {
        CtxMutability::Shared => quote! {
            impl #host_crate::#has_trait for Backends {
                fn #ctx_method(&self) -> &dyn #host_crate::#ctx_trait {
                    &self.#field
                }
            }
        },
        CtxMutability::Exclusive => quote! {
            impl #host_crate::#has_trait for Backends {
                fn #ctx_method(&mut self) -> &mut dyn #host_crate::#ctx_trait {
                    &mut self.#field
                }
            }
        },
    }
}

fn service_stem(path: &Path) -> String {
    let Some(segment) = path.segments.last() else {
        return String::new();
    };

    let name = segment.ident.to_string();
    name.strip_prefix("Wasi").unwrap_or(name.as_str()).to_string()
}

fn has_trait(path: &Path) -> Ident {
    format_ident!("Has{}", service_stem(path))
}

fn ctx_trait(path: &Path) -> Ident {
    format_ident!("Wasi{}Ctx", service_stem(path))
}

fn ctx_method(path: &Path) -> Ident {
    format_ident!("{}_ctx", service_stem(path).to_lowercase())
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
    use crate::runtime::parse::HostEntry;

    fn path(name: &str) -> Path {
        syn::parse_str(name).expect("valid path")
    }

    fn host_entry(host: &str, backend: &str) -> HostEntry {
        HostEntry {
            host: path(host),
            backend: path(backend),
        }
    }

    #[test]
    fn derives_accessor_names() {
        for (input, has, ctx, method) in [
            ("WasiJsonDb", "HasJsonDb", "WasiJsonDbCtx", "jsondb_ctx"),
            ("WasiWebSocket", "HasWebSocket", "WasiWebSocketCtx", "websocket_ctx"),
            ("WasiKeyValue", "HasKeyValue", "WasiKeyValueCtx", "keyvalue_ctx"),
        ] {
            let host = path(input);
            assert_eq!(has_trait(&host).to_string(), has);
            assert_eq!(ctx_trait(&host).to_string(), ctx);
            assert_eq!(ctx_method(&host).to_string(), method);
        }
    }

    #[test]
    fn derives_crate_ident() {
        assert_eq!(wasi_ident(&path("WasiJsonDb")).to_string(), "omnia_wasi_jsondb");
        assert_eq!(wasi_ident(&path("WasiWebSocket")).to_string(), "omnia_wasi_websocket");
    }

    #[test]
    fn derives_field_ident() {
        assert_eq!(field_ident(&path("HttpDefault")).to_string(), "http_default");
        assert_eq!(field_ident(&path("KeyValueDefault")).to_string(), "key_value_default");
    }

    #[test]
    fn dedupes_backends() {
        let config = Config {
            command: false,
            host_entries: vec![
                host_entry("WasiOtel", "OtelDefault"),
                host_entry("WasiHttp", "HttpDefault"),
                host_entry("WasiHttp", "HttpDefault"),
            ],
        };

        let fields = config.backend_fields();

        let idents: Vec<_> = fields.iter().map(|field| field.ident.to_string()).collect();
        let types: Vec<_> =
            fields.iter().map(|field| field.ty.get_ident().unwrap().to_string()).collect();

        assert_eq!(idents, ["otel_default", "http_default"]);
        assert_eq!(types, ["OtelDefault", "HttpDefault"]);
    }
}
