//! # Codegen for the runtime macro.
//!
//! Generates the token stream fragments required to expand the runtime macro.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Ident, Path};

use crate::runtime::parse::{Config, HostEntry, Mode};

// Token fragments needed to expand the runtime macro.
pub struct Codegen {
    pub mode: Mode,
    pub host_types: Vec<Path>,
    pub server_types: Vec<Path>,
    pub backends_ty: TokenStream,
    pub backends_def: TokenStream,
}

impl From<&Config> for Codegen {
    fn from(config: &Config) -> Self {
        let host_entries = &config.host_entries;
        let host_types: Vec<Path> = host_entries.iter().map(|entry| entry.host.clone()).collect();
        let server_types: Vec<Path> =
            host_types.iter().filter(|host| is_server(host)).cloned().collect();

        let (backends_ty, backends_def) = emit_backends(host_entries);

        Self {
            mode: config.mode,
            host_types,
            server_types,
            backends_ty,
            backends_def,
        }
    }
}

fn is_server(host: &Path) -> bool {
    matches!(
        host.segments.last().map(|segment| segment.ident.to_string()).as_deref(),
        Some("WasiHttp" | "WasiMessaging" | "WasiWebSocket")
    )
}

fn emit_backends(host_entries: &[HostEntry]) -> (TokenStream, TokenStream) {
    let mut backends: Vec<Path> = host_entries.iter().map(|e| e.backend.clone()).collect();
    backends.dedup_by(|a, b| path_key(a) == path_key(b));

    let (idents, types): (Vec<Ident>, Vec<Path>) =
        backends.into_iter().map(|ty| (field_ident(&ty), ty)).unzip();

    if idents.is_empty() {
        return (quote! { () }, quote! {});
    }

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
                        #(<#types as Backend>::connect(),)*
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
            ("WasiDocStore", "HasDocStore", "WasiDocStoreCtx", "docstore_ctx"),
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
        assert_eq!(wasi_ident(&path("WasiDocStore")).to_string(), "omnia_wasi_docstore");
        assert_eq!(wasi_ident(&path("WasiWebSocket")).to_string(), "omnia_wasi_websocket");
    }

    #[test]
    fn derives_field_ident() {
        assert_eq!(field_ident(&path("HttpDefault")).to_string(), "http_default");
        assert_eq!(field_ident(&path("KeyValueDefault")).to_string(), "key_value_default");
    }

    #[test]
    fn dedupes_backends() {
        let entries = [
            host_entry("WasiOtel", "OtelDefault"),
            host_entry("WasiHttp", "HttpDefault"),
            host_entry("WasiHttp", "HttpDefault"),
        ];
        let (ty, def) = emit_backends(&entries);
        let def = def.to_string();

        assert_eq!(ty.to_string(), "Backends");

        let struct_start = def.find("struct Backends").expect("Backends struct");
        let struct_end = def.find("impl omnia").expect("Backends impl");
        let struct_body = &def[struct_start..struct_end];

        assert_eq!(struct_body.matches("otel_default").count(), 1);
        assert_eq!(struct_body.matches("http_default").count(), 1);
        assert!(
            struct_body.find("otel_default").unwrap() < struct_body.find("http_default").unwrap()
        );
    }

    #[test]
    fn empty_host_entries() {
        let (ty, def) = emit_backends(&[]);
        assert_eq!(ty.to_string(), "()");
        assert!(def.is_empty());
    }
}
