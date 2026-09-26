//! # Parse
//!
//! Parses the runtime macro token stream input into structured values.

use proc_macro2::Span;
use quote::ToTokens;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Expr, Ident, Path, Result, Token};

/// Deployment drive mode parsed from `runtime!({ ... })`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Server,
    Command,
}

impl Mode {
    /// The `omnia::Mode` path this mode expands to.
    pub fn tokens(self) -> proc_macro2::TokenStream {
        match self {
            Self::Server => quote::quote!(omnia::Mode::Server),
            Self::Command => quote::quote!(omnia::Mode::Command),
        }
    }
}

/// Configuration for the runtime macro.
pub struct Config {
    pub mode: Mode,
    pub host_entries: Vec<HostEntry>,
    /// The `manifest:` path expression, when the deployment is a manifest
    /// file rather than inline keys.
    pub manifest_file: Option<Expr>,
    pub manifest: ManifestSpec,
}

/// One `Host: Backend` wiring from the `hosts: { ... }` block, optionally
/// carrying compiled-in connect options: `Host: Backend(options)` lowers to
/// `Backend::connect_with(options)` instead of the env-sourced
/// `Backend::connect()`.
pub struct HostEntry {
    pub host: Path,
    pub backend: Path,
    pub options: Option<Expr>,
}

/// Inline manifest keys (`guests`, `registries`, `mounts`) parsed from
/// `runtime!({ ... })`; mirrors the `omnia::Manifest` schema.
#[derive(Default)]
pub struct ManifestSpec {
    /// The `guests:` list — every component the deployment may run.
    pub guests: Vec<GuestSpec>,
    pub mounts: Vec<MountSpec>,
    /// The `registries:` expression — the wasm-pkg configuration (TOML) that
    /// routes package sources, typically an `include_str!`.
    pub registries: Option<Expr>,
}

impl ManifestSpec {
    pub const fn is_empty(&self) -> bool {
        self.guests.is_empty() && self.mounts.is_empty() && self.registries.is_none()
    }
}

/// One `{ path: ..., name: ..., routes: { ... }, command: true }` (or
/// `{ package: ..., digest: ... }`) guest entry.
pub struct GuestSpec {
    pub source: GuestSource,
    pub routes: GuestRoutesSpec,
    pub command: bool,
    /// Span of the `command:` key, for cross-key diagnostics.
    pub command_span: Option<Span>,
    /// The `digest:` pin, decoded from its `sha256:<hex>` literal.
    pub digest: Option<[u8; DIGEST_LEN]>,
}

/// Where a guest entry's component comes from.
pub enum GuestSource {
    /// A `path:` embedded with `include_bytes!` — a string literal or a macro
    /// such as `concat!(env!(..), ..)`; the file's stem names the guest
    /// unless `name:` does.
    Embedded { path: Expr, name: Option<Expr> },
    /// A `package:` reference the runtime fetches at the guest's first use;
    /// the reference without its version names the guest unless `name:`
    /// does.
    Package { reference: Expr, name: Option<Expr> },
}

const DIGEST_LEN: usize = 32;

/// One `{ name: ..., path: ..., writable: ... }` mount entry.
pub struct MountSpec {
    pub name: Expr,
    pub path: Expr,
    pub writable: Option<Expr>,
}

/// Per-trigger route pattern lists from a guest entry's `routes: { ... }`
/// block; the containing guest is the implicit target.
#[derive(Default)]
pub struct GuestRoutesSpec {
    pub http: Vec<Expr>,
    pub messaging: Vec<Expr>,
    pub websocket: Vec<Expr>,
}

impl Parse for Config {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut mode = Mode::default();
        let mut host_entries = Vec::new();
        let mut manifest_file = None;
        let mut manifest = ManifestSpec::default();
        let mut manifest_span: Option<Span> = None;
        let mut inline_span: Option<Span> = None;

        let settings;
        syn::braced!(settings in input);
        let settings = Punctuated::<Opt, Token![,]>::parse_terminated(&settings)?;

        let mut seen: Vec<&'static str> = Vec::new();
        for setting in settings.into_pairs() {
            let Opt { name, span, value } = setting.into_value();
            if seen.contains(&name) {
                return Err(syn::Error::new(span, format!("duplicate `{name}:` key")));
            }
            seen.push(name);
            match value {
                OptValue::Mode(m) => mode = m,
                OptValue::Hosts(h) => host_entries = h,
                OptValue::Manifest(m) => {
                    manifest_file = Some(m);
                    manifest_span = Some(span);
                }
                OptValue::Guests(g) => {
                    manifest.guests = g;
                    inline_span.get_or_insert(span);
                }
                OptValue::Registries(r) => {
                    manifest.registries = Some(r);
                    inline_span.get_or_insert(span);
                }
                OptValue::Mounts(m) => {
                    manifest.mounts = m;
                    inline_span.get_or_insert(span);
                }
            }
        }

        let config = Self {
            mode,
            host_entries,
            manifest_file,
            manifest,
        };
        config.validate(&KeySpans {
            manifest: manifest_span,
            inline: inline_span,
        })?;
        Ok(config)
    }
}

// kept out of `Config`, since they matter only for diagnostics
struct KeySpans {
    manifest: Option<Span>,
    inline: Option<Span>,
}

impl Config {
    fn validate(&self, spans: &KeySpans) -> syn::Result<()> {
        if let (Some(_), Some(inline)) = (spans.manifest, spans.inline) {
            return Err(syn::Error::new(
                inline,
                "`manifest:` and the inline keys (`guests`, `registries`, `mounts`) are mutually \
                 exclusive; declare the deployment in the manifest file",
            ));
        }

        // rows sharing a backend share one connection, so their options must agree
        let mut options_seen: std::collections::HashMap<String, Option<String>> =
            std::collections::HashMap::new();
        for entry in &self.host_entries {
            let backend = entry.backend.to_token_stream().to_string();
            let options = entry.options.as_ref().map(|expr| expr.to_token_stream().to_string());
            if *options_seen.entry(backend).or_insert_with(|| options.clone()) != options {
                let span =
                    entry.options.as_ref().map_or_else(|| entry.backend.span(), Spanned::span);
                return Err(syn::Error::new(
                    span,
                    "hosts rows sharing a backend share one connection; their connect \
                     options must be identical (or omitted on every row)",
                ));
            }
        }

        let mut marked: Option<Span> = None;
        for guest in &self.manifest.guests {
            let Some(span) = guest.command_span else {
                continue;
            };
            if self.mode != Mode::Command {
                return Err(syn::Error::new(
                    span,
                    "`command: true` requires `mode: command` (it only routes command mode)",
                ));
            }
            if marked.replace(span).is_some() {
                return Err(syn::Error::new(
                    span,
                    "multiple guests marked `command: true`; at most one guest may be the \
                     command guest",
                ));
            }
        }

        Ok(())
    }
}

mod kw {
    syn::custom_keyword!(mode);
    syn::custom_keyword!(hosts);
    syn::custom_keyword!(manifest);
    syn::custom_keyword!(guests);
    syn::custom_keyword!(registries);
    syn::custom_keyword!(mounts);
}

// the span lets `Config::parse` reject a duplicate with a pointed diagnostic
struct Opt {
    name: &'static str,
    span: Span,
    value: OptValue,
}

enum OptValue {
    Mode(Mode),
    Hosts(Vec<HostEntry>),
    Manifest(Expr),
    Guests(Vec<GuestSpec>),
    Registries(Expr),
    Mounts(Vec<MountSpec>),
}

impl Parse for Opt {
    fn parse(input: ParseStream) -> Result<Self> {
        let l = input.lookahead1();
        let (name, span, value) = if l.peek(kw::mode) {
            let key = input.parse::<kw::mode>()?;
            input.parse::<Token![:]>()?;
            ("mode", key.span, OptValue::Mode(parse_mode(input)?))
        } else if l.peek(kw::hosts) {
            let key = input.parse::<kw::hosts>()?;
            input.parse::<Token![:]>()?;
            let list;
            syn::braced!(list in input);
            ("hosts", key.span, OptValue::Hosts(parse_host_entries(&list)?))
        } else if l.peek(kw::manifest) {
            let key = input.parse::<kw::manifest>()?;
            input.parse::<Token![:]>()?;
            ("manifest", key.span, OptValue::Manifest(input.parse()?))
        } else if l.peek(kw::guests) {
            let key = input.parse::<kw::guests>()?;
            input.parse::<Token![:]>()?;
            ("guests", key.span, OptValue::Guests(parse_bracketed_list(input)?))
        } else if l.peek(kw::registries) {
            let key = input.parse::<kw::registries>()?;
            input.parse::<Token![:]>()?;
            ("registries", key.span, OptValue::Registries(input.parse()?))
        } else if l.peek(kw::mounts) {
            let key = input.parse::<kw::mounts>()?;
            input.parse::<Token![:]>()?;
            ("mounts", key.span, OptValue::Mounts(parse_bracketed_list(input)?))
        } else {
            return Err(l.error());
        };
        Ok(Self { name, span, value })
    }
}

fn parse_mode(input: ParseStream) -> Result<Mode> {
    let ident: Ident = input.parse()?;
    match ident.to_string().as_str() {
        "server" => Ok(Mode::Server),
        "command" => Ok(Mode::Command),
        other => Err(syn::Error::new(
            ident.span(),
            format!("expected `server` or `command`, got `{other}`"),
        )),
    }
}

impl Parse for HostEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let host = input.parse::<Path>()?;
        input.parse::<Token![:]>()?;
        let backend = input.parse::<Path>()?;
        let options = if input.peek(syn::token::Paren) {
            let args;
            let paren = syn::parenthesized!(args in input);
            if args.is_empty() {
                return Err(syn::Error::new(
                    paren.span.join(),
                    "empty connect options; drop the `()` to connect from the environment",
                ));
            }
            let expr = args.parse::<Expr>()?;
            if !args.is_empty() {
                return Err(args.error("expected a single connect-options expression"));
            }
            Some(expr)
        } else {
            None
        };
        Ok(Self {
            host,
            backend,
            options,
        })
    }
}

fn parse_host_entries(input: ParseStream) -> Result<Vec<HostEntry>> {
    Ok(Punctuated::<HostEntry, Token![,]>::parse_terminated(input)?.into_iter().collect())
}

fn parse_bracketed_list<T: Parse>(input: ParseStream) -> Result<Vec<T>> {
    let list;
    syn::bracketed!(list in input);
    Ok(Punctuated::<T, Token![,]>::parse_terminated(&list)?.into_iter().collect())
}

// Hands each key and the stream at its value to `field`, refusing repeats;
// returns the brace span for missing-key diagnostics.
fn parse_kv_block(
    input: ParseStream, mut field: impl FnMut(&Ident, ParseStream) -> Result<()>,
) -> Result<Span> {
    let content;
    let brace = syn::braced!(content in input);
    let mut seen: Vec<String> = Vec::new();
    while !content.is_empty() {
        let key: Ident = content.parse()?;
        let name = key.to_string();
        if seen.contains(&name) {
            return Err(syn::Error::new(key.span(), format!("duplicate `{name}:` key")));
        }
        seen.push(name);
        content.parse::<Token![:]>()?;
        field(&key, &content)?;
        if !content.is_empty() {
            content.parse::<Token![,]>()?;
        }
    }
    Ok(brace.span.join())
}

impl Parse for GuestSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut path: Option<Expr> = None;
        let mut package: Option<(Expr, Span)> = None;
        let mut name = None;
        let mut routes = GuestRoutesSpec::default();
        let mut command = false;
        let mut command_span = None;
        let mut digest = None;

        let span = parse_kv_block(input, |key, value| {
            match key.to_string().as_str() {
                "path" => path = Some(parse_embeddable(value)?),
                "package" => package = Some((value.parse()?, key.span())),
                "name" => name = Some(value.parse()?),
                "routes" => routes = value.parse()?,
                "command" => {
                    let lit: syn::LitBool = value.parse()?;
                    command = lit.value();
                    command_span = command.then(|| key.span());
                }
                "digest" => digest = Some(parse_digest(value)?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown guest key `{other}`; expected `path` or `package`, `name`, \
                             `routes`, `command`, or `digest`"
                        ),
                    ));
                }
            }
            Ok(())
        })?;

        let source = match (path, package) {
            (Some(_), Some((_, span))) => {
                return Err(syn::Error::new(
                    span,
                    "`path:` and `package:` are mutually exclusive; a guest has one source",
                ));
            }
            (Some(path), None) => GuestSource::Embedded { path, name },
            (None, Some((reference, _))) => GuestSource::Package { reference, name },
            (None, None) => {
                return Err(syn::Error::new(span, "guest entry is missing `path` (or `package`)"));
            }
        };

        Ok(Self {
            source,
            routes,
            command,
            command_span,
            digest,
        })
    }
}

// a malformed pin is refused here, pointed at the key, rather than at start-up
fn parse_digest(input: ParseStream) -> Result<[u8; DIGEST_LEN]> {
    let lit: syn::LitStr = input.parse()?;
    let value = lit.value();
    let malformed = |detail: &str| {
        syn::Error::new(
            lit.span(),
            format!("`digest:` must be `sha256:<{} hex characters>`: {detail}", DIGEST_LEN * 2),
        )
    };
    let hex = value.strip_prefix("sha256:").ok_or_else(|| malformed("missing `sha256:`"))?;
    if hex.len() != DIGEST_LEN * 2 {
        return Err(malformed(&format!("got {} hex characters", hex.len())));
    }
    let mut bytes = [0; DIGEST_LEN];
    let (pairs, _) = hex.as_bytes().as_chunks::<2>();
    for (byte, &[high, low]) in bytes.iter_mut().zip(pairs) {
        let (Some(high), Some(low)) = (nibble(high), nibble(low)) else {
            return Err(malformed("not hexadecimal"));
        };
        *byte = (high << 4) | low;
    }
    Ok(bytes)
}

const fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

// `include_bytes!` takes a literal or a macro expanding to one; anything else
// is refused here, where the diagnostic can name the key
fn parse_embeddable(input: ParseStream) -> Result<Expr> {
    let expr: Expr = input.parse()?;
    match &expr {
        Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(_),
            ..
        })
        | Expr::Macro(_) => Ok(expr),
        other => Err(syn::Error::new(
            other.span(),
            "`path:` is embedded with `include_bytes!`, so it must be a string literal or a \
             macro such as `concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/guest.wasm\")`",
        )),
    }
}

impl Parse for MountSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut name = None;
        let mut path = None;
        let mut writable = None;

        let span = parse_kv_block(input, |key, value| {
            match key.to_string().as_str() {
                "name" => name = Some(value.parse()?),
                "path" => path = Some(value.parse()?),
                "writable" => writable = Some(value.parse()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown mount key `{other}`; expected `name`, `path`, or `writable`"
                        ),
                    ));
                }
            }
            Ok(())
        })?;

        let missing = |key| syn::Error::new(span, format!("mount entry is missing `{key}`"));
        Ok(Self {
            name: name.ok_or_else(|| missing("name"))?,
            path: path.ok_or_else(|| missing("path"))?,
            writable,
        })
    }
}

impl Parse for GuestRoutesSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut routes = Self::default();

        parse_kv_block(input, |key, value| {
            match key.to_string().as_str() {
                "http" => routes.http = parse_bracketed_list(value)?,
                "messaging" => routes.messaging = parse_bracketed_list(value)?,
                "websocket" => routes.websocket = parse_bracketed_list(value)?,
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown route trigger `{other}`; expected `http`, `messaging`, or \
                             `websocket`"
                        ),
                    ));
                }
            }
            Ok(())
        })?;

        Ok(routes)
    }
}
