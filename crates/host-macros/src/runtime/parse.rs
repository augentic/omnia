//! # Parse
//!
//! Parses the runtime macro token stream input into structured values.

use quote::format_ident;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitBool, Path, Result, Token};

/// Configuration for the runtime macro.
pub struct Config {
    pub command: bool,
    pub hosts: Vec<Host>,
    pub backends: Vec<Path>,
}

pub struct Hosts(pub Vec<Host>);

/// Information about a WASI host and its configuration.
pub struct Host {
    pub type_: Path,
    pub backend: Option<Path>,
}

impl Parse for Config {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut command = false;
        let mut hosts = Hosts(Vec::new());

        let settings;
        syn::braced!(settings in input);
        let settings = Punctuated::<Opt, Token![,]>::parse_terminated(&settings)?;

        for setting in settings.into_pairs() {
            match setting.into_value() {
                Opt::Command(c) => command = c,
                Opt::Host(h) => hosts = h,
            }
        }

        // deduplicate backends (skipping backend-less hosts)
        let mut backends: Vec<Path> = vec![];
        for host in &hosts.0 {
            let Some(backend) = &host.backend else {
                continue;
            };
            if backends.iter().any(|b| b.get_ident() == backend.get_ident()) {
                continue;
            }
            backends.push(backend.clone());
        }

        Ok(Self {
            command,
            hosts: hosts.0,
            backends,
        })
    }
}

mod kw {
    syn::custom_keyword!(command);
    syn::custom_keyword!(hosts);
}

#[allow(clippy::large_enum_variant)]
enum Opt {
    Command(bool),
    Host(Hosts),
}

impl Parse for Opt {
    fn parse(input: ParseStream) -> Result<Self> {
        let l = input.lookahead1();
        if l.peek(kw::command) {
            input.parse::<kw::command>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Command(input.parse::<LitBool>()?.value))
        } else if l.peek(kw::hosts) {
            input.parse::<kw::hosts>()?;
            input.parse::<Token![:]>()?;
            let list;
            syn::braced!(list in input);
            Ok(Self::Host(list.parse()?))
        } else {
            Err(l.error())
        }
    }
}

impl Parse for Hosts {
    fn parse(input: ParseStream) -> Result<Self> {
        let hosts = Punctuated::<Host, Token![,]>::parse_terminated(input)?;
        Ok(Self(hosts.into_iter().collect()))
    }
}

impl Parse for Host {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let type_ = input.parse::<Path>()?;
        // `Host: Backend` (a backend-backed host) or bare `Host` (backend-less).
        let backend = if input.peek(Token![:]) {
            input.parse::<Token![:]>()?;
            Some(input.parse::<Path>()?)
        } else {
            None
        };
        Ok(Self { type_, backend })
    }
}

/// Derives a `snake_case` field name from a backend type's final path segment
/// (e.g. `HttpDefault` -> `http_default`).
pub fn field_ident(path: &Path) -> Ident {
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

/// Derives a host crate's module ident from a host type's final path segment
/// (e.g. `WasiHttp` -> `omnia_wasi_http`), naming the host crate whose bundle
/// accessor trait the generated `Backends` impl satisfies.
pub fn wasi_ident(path: &Path) -> Ident {
    let Some(segment) = path.segments.last() else {
        return format_ident!("wasi");
    };

    let name = segment.ident.to_string().replace("Wasi", "omnia_wasi_").to_lowercase();
    format_ident!("{name}")
}

/// Recovers a host type's service stem by stripping the `Wasi` prefix from its
/// final path segment (e.g. `WasiJsonDb` -> `JsonDb`).
fn service_stem(path: &Path) -> String {
    let Some(segment) = path.segments.last() else {
        return String::new();
    };

    let name = segment.ident.to_string();
    name.strip_prefix("Wasi").unwrap_or(name.as_str()).to_string()
}

/// Derives the bundle accessor trait ident (e.g. `WasiJsonDb` -> `HasJsonDb`).
pub fn has_trait(path: &Path) -> Ident {
    format_ident!("Has{}", service_stem(path))
}

/// Derives the backend context trait ident (e.g. `WasiJsonDb` -> `WasiJsonDbCtx`).
pub fn ctx_trait(path: &Path) -> Ident {
    format_ident!("Wasi{}Ctx", service_stem(path))
}

/// Derives the bundle accessor method ident (e.g. `WasiJsonDb` -> `jsondb_ctx`).
pub fn ctx_method(path: &Path) -> Ident {
    format_ident!("{}_ctx", service_stem(path).to_lowercase())
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
