//! # Parse
//!
//! Parses the runtime macro token stream input into structured values.

use proc_macro2::Span;
use quote::ToTokens;
use syn::ext::IdentExt as _;
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
    #[allow(clippy::struct_field_names)]
    pub config_file: Option<Expr>,
    pub manifest: ManifestSpec,
    /// Whether the deployment declares plugin locations — inline through the
    /// `plugin:` block's `locations:` list, or in the config file when a
    /// bare `plugin:` block accompanies `config:`. Either links the
    /// `WasiPlugins` loader host and installs the locations, which requires
    /// `omnia`'s `plugin` feature; a `link:`-only invocation never references
    /// the loader.
    pub link_loader: bool,
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

/// Inline manifest keys (`link` interfaces, `plugin` locations, `guests`,
/// `mounts`, `env`) parsed from `runtime!({ ... })`; mirrors the
/// `omnia::Manifest` schema.
#[derive(Default)]
pub struct ManifestSpec {
    pub interfaces: Vec<Expr>,
    pub locations: Vec<LocationSpec>,
    pub guests: Vec<GuestSpec>,
    pub mounts: Vec<MountSpec>,
    pub env: Vec<EnvSpec>,
}

/// One `NAME: value` entry of the `env: { ... }` block: a guest environment
/// default. The name is written as an identifier or a string literal and
/// lowered to a string literal; the value is any expression yielding a string.
pub struct EnvSpec {
    pub name: syn::LitStr,
    pub value: Expr,
}

/// The `link: { interfaces: [...] }` block: the deployment's host-mediated
/// interface set.
#[derive(Default)]
pub struct LinkSpec {
    pub interfaces: Vec<Expr>,
}

/// The `plugin: { locations: [...] }` block: the deployment's loader
/// acquisition locations.
#[derive(Default)]
pub struct PluginSpec {
    pub locations: Vec<LocationSpec>,
}

/// One `locations:` entry, discriminated by the keys present.
pub enum LocationSpec {
    /// `{ name: ..., path: ... }` — one named root the deployment opens at
    /// startup for path loads.
    Path {
        /// The location name path loads resolve against.
        name: Expr,
        /// The host directory backing the location.
        path: Expr,
    },
    /// `{ registry: ..., config: ... }` — the deployment's default registry
    /// endpoint and, optionally, the wasm-pkg client configuration (TOML)
    /// routing namespaces and packages to other registries.
    Registry {
        /// The default endpoint.
        registry: Expr,
        /// The wasm-pkg configuration, typically an `include_str!`.
        config: Option<Expr>,
    },
}

impl ManifestSpec {
    pub const fn is_empty(&self) -> bool {
        self.interfaces.is_empty()
            && self.locations.is_empty()
            && self.guests.is_empty()
            && self.mounts.is_empty()
            && self.env.is_empty()
    }
}

/// One `{ id: ..., source: ..., routes: { ... }, command: true }` guest entry.
pub struct GuestSpec {
    pub id: Expr,
    pub source: Expr,
    pub routes: GuestRoutesSpec,
    pub command: bool,
    /// Span of the `command:` key, for cross-key diagnostics.
    pub command_span: Option<Span>,
}

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
        let mut config_file = None;
        let mut manifest = ManifestSpec::default();
        let mut plugin_span: Option<Span> = None;
        let mut link_span: Option<Span> = None;
        let mut config_span: Option<Span> = None;
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
                OptValue::Config(c) => {
                    config_file = Some(c);
                    config_span = Some(span);
                }
                OptValue::Link(p) => {
                    link_span = Some(span);
                    if !p.interfaces.is_empty() {
                        inline_span.get_or_insert(span);
                    }
                    manifest.interfaces = p.interfaces;
                }
                OptValue::Plugin(p) => {
                    plugin_span = Some(span);
                    // Locations are manifest data, so they conflict with
                    // `config:`; a bare `plugin: {}` is only meaningful beside
                    // `config:`, where it opts into the loader over the TOML's
                    // `[[plugin.location]]` entries.
                    if !p.locations.is_empty() {
                        inline_span.get_or_insert(span);
                    }
                    manifest.locations = p.locations;
                }
                OptValue::Guests(g) => {
                    manifest.guests = g;
                    inline_span.get_or_insert(span);
                }
                OptValue::Mounts(m) => {
                    manifest.mounts = m;
                    inline_span.get_or_insert(span);
                }
                OptValue::Env(e) => {
                    manifest.env = e;
                    inline_span.get_or_insert(span);
                }
            }
        }

        // A bare `link: {}` declares nothing: the seam needs no host linking,
        // so an empty block would expand to nothing at all.
        if let Some(span) = link_span
            && manifest.interfaces.is_empty()
        {
            return Err(syn::Error::new(
                span,
                "`link: {}` declares nothing; add `interfaces: [\"ns:pkg/iface\"]`",
            ));
        }

        // A bare `plugin: {}` declares nothing inline and, without `config:`,
        // has no TOML to defer to — it would expand to nothing at all.
        if let Some(span) = plugin_span
            && manifest.locations.is_empty()
            && config_file.is_none()
        {
            return Err(syn::Error::new(
                span,
                "`plugin: {}` declares nothing; add `locations:`, or pair it with `config:` to \
                 install the config file's `[[plugin.location]]` entries",
            ));
        }

        // Only declared locations opt into the loader: inline ones
        // (`PluginSpec::validate` already refuses an empty list), or the
        // config file's when a `plugin:` block accompanies `config:`. A
        // `link:`-only invocation is plain manifest data.
        let link_loader =
            !manifest.locations.is_empty() || (plugin_span.is_some() && config_file.is_some());
        let config = Self {
            mode,
            host_entries,
            config_file,
            manifest,
            link_loader,
        };
        config.validate(&KeySpans {
            config: config_span,
            inline: inline_span,
        })?;
        Ok(config)
    }
}

/// Spans of the keys that participate in cross-key validation, kept out of
/// [`Config`] itself since they matter only for diagnostics.
struct KeySpans {
    config: Option<Span>,
    inline: Option<Span>,
}

impl Config {
    fn validate(&self, spans: &KeySpans) -> syn::Result<()> {
        if let (Some(_), Some(inline)) = (spans.config, spans.inline) {
            return Err(syn::Error::new(
                inline,
                "`config:` and inline manifest keys (`link` interfaces, `plugin` locations, \
                 `guests`, `mounts`, `env`) are mutually exclusive; declare `[[plugin.location]]` \
                 entries in the config file",
            ));
        }

        // Rows sharing a backend type share one connection, so their connect
        // options must agree token-for-token — or be absent on every row.
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
    syn::custom_keyword!(config);
    syn::custom_keyword!(plugin);
    syn::custom_keyword!(guests);
    syn::custom_keyword!(mounts);
    syn::custom_keyword!(link);
    syn::custom_keyword!(env);
}

/// One `key: value` setting, tagged with its key name and span so
/// `Config::parse` can reject duplicates with a pointed diagnostic.
struct Opt {
    name: &'static str,
    span: Span,
    value: OptValue,
}

enum OptValue {
    Mode(Mode),
    Hosts(Vec<HostEntry>),
    Config(Expr),
    Link(LinkSpec),
    Plugin(PluginSpec),
    Guests(Vec<GuestSpec>),
    Mounts(Vec<MountSpec>),
    Env(Vec<EnvSpec>),
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
        } else if l.peek(kw::config) {
            let key = input.parse::<kw::config>()?;
            input.parse::<Token![:]>()?;
            ("config", key.span, OptValue::Config(input.parse()?))
        } else if l.peek(kw::link) {
            let key = input.parse::<kw::link>()?;
            input.parse::<Token![:]>()?;
            if input.peek(syn::token::Bracket) {
                return Err(syn::Error::new(
                    key.span,
                    "the `link:` key takes a block: `link: { interfaces: [\"ns:pkg/iface\"] }`",
                ));
            }
            ("link", key.span, OptValue::Link(input.parse()?))
        } else if l.peek(kw::plugin) {
            let key = input.parse::<kw::plugin>()?;
            input.parse::<Token![:]>()?;
            if input.peek(syn::token::Bracket) {
                return Err(syn::Error::new(
                    key.span,
                    "the `plugin:` key takes a block: `plugin: { locations: [...] }`",
                ));
            }
            ("plugin", key.span, OptValue::Plugin(input.parse()?))
        } else if l.peek(kw::guests) {
            let key = input.parse::<kw::guests>()?;
            input.parse::<Token![:]>()?;
            ("guests", key.span, OptValue::Guests(parse_bracketed_list(input)?))
        } else if l.peek(kw::mounts) {
            let key = input.parse::<kw::mounts>()?;
            input.parse::<Token![:]>()?;
            ("mounts", key.span, OptValue::Mounts(parse_bracketed_list(input)?))
        } else if l.peek(kw::env) {
            let key = input.parse::<kw::env>()?;
            input.parse::<Token![:]>()?;
            if input.peek(syn::token::Bracket) {
                return Err(syn::Error::new(
                    key.span,
                    "the `env:` key takes a block: `env: { RUST_LOG: \"my_sdk=info\" }`",
                ));
            }
            ("env", key.span, OptValue::Env(parse_env(input, key.span)?))
        } else {
            return Err(l.error());
        };
        Ok(Self { name, span, value })
    }
}

/// Parse the `env: { NAME: value, ... }` block: each name an identifier or a
/// string literal, each value an expression; a repeated name and an empty
/// block are refused.
fn parse_env(input: ParseStream, key_span: Span) -> Result<Vec<EnvSpec>> {
    let content;
    syn::braced!(content in input);
    let mut entries: Vec<EnvSpec> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    while !content.is_empty() {
        let name: syn::LitStr = if content.peek(syn::LitStr) {
            content.parse()?
        } else {
            let ident: Ident = content.call(Ident::parse_any)?;
            syn::LitStr::new(&ident.to_string(), ident.span())
        };
        let text = name.value();
        if seen.contains(&text) {
            return Err(syn::Error::new(name.span(), format!("duplicate `{text}:` entry")));
        }
        seen.push(text);
        content.parse::<Token![:]>()?;
        let value: Expr = content.parse()?;
        entries.push(EnvSpec { name, value });
        if !content.is_empty() {
            content.parse::<Token![,]>()?;
        }
    }
    if entries.is_empty() {
        return Err(syn::Error::new(
            key_span,
            "`env: {}` declares nothing; add `NAME: \"value\"` entries",
        ));
    }
    Ok(entries)
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

/// Parse `[ item, item, ... ]` where each item implements [`Parse`].
fn parse_bracketed_list<T: Parse>(input: ParseStream) -> Result<Vec<T>> {
    let list;
    syn::bracketed!(list in input);
    Ok(Punctuated::<T, Token![,]>::parse_terminated(&list)?.into_iter().collect())
}

/// Parse a braced `{ key: value, ... }` block, handing each key (and the
/// stream positioned at its value) to `field`. Repeated keys are refused
/// with a pointed diagnostic. Returns the brace span for missing-key
/// diagnostics.
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
        let mut id = None;
        let mut source = None;
        let mut routes = GuestRoutesSpec::default();
        let mut command = false;
        let mut command_span = None;

        let span = parse_kv_block(input, |key, value| {
            match key.to_string().as_str() {
                "id" => id = Some(value.parse()?),
                "source" => source = Some(value.parse()?),
                "routes" => routes = value.parse()?,
                "command" => {
                    let lit: syn::LitBool = value.parse()?;
                    command = lit.value();
                    command_span = command.then(|| key.span());
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown guest key `{other}`; expected `id`, `source`, `routes`, \
                             or `command`"
                        ),
                    ));
                }
            }
            Ok(())
        })?;

        let missing = |key| syn::Error::new(span, format!("guest entry is missing `{key}`"));
        Ok(Self {
            id: id.ok_or_else(|| missing("id"))?,
            source: source.ok_or_else(|| missing("source"))?,
            routes,
            command,
            command_span,
        })
    }
}

impl Parse for LinkSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut spec = Self::default();

        parse_kv_block(input, |key, value| {
            match key.to_string().as_str() {
                "interfaces" => spec.interfaces = parse_bracketed_list(value)?,
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown link key `{other}`; expected `interfaces`"),
                    ));
                }
            }
            Ok(())
        })?;

        Ok(spec)
    }
}

impl Parse for PluginSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut spec = Self::default();
        let mut locations_span = None;

        parse_kv_block(input, |key, value| {
            match key.to_string().as_str() {
                "locations" => {
                    spec.locations = parse_bracketed_list(value)?;
                    locations_span = Some(key.span());
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown plugin key `{other}`; expected `locations`"),
                    ));
                }
            }
            Ok(())
        })?;

        spec.validate(locations_span)?;
        Ok(spec)
    }
}

impl PluginSpec {
    /// The block's cross-key refusals, spanned to the offending key.
    fn validate(&self, locations_span: Option<Span>) -> Result<()> {
        if let Some(span) = locations_span
            && self.locations.is_empty()
        {
            return Err(syn::Error::new(
                span,
                "`locations:` is empty; declare at least one `{ name, path }` or \
                 `{ registry: ... }` entry",
            ));
        }

        let mut registries = self.locations.iter().filter_map(|location| match location {
            LocationSpec::Registry { registry, .. } => Some(registry.span()),
            LocationSpec::Path { .. } => None,
        });
        if let Some(second) = registries.nth(1) {
            return Err(syn::Error::new(
                second,
                "more than one `registry` location; a deployment declares one default \
                 registry (a load's own location may still override the endpoint)",
            ));
        }

        Ok(())
    }
}

impl Parse for LocationSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut name = None;
        let mut path = None;
        let mut registry = None;
        let mut config = None;

        let span = parse_kv_block(input, |key, value| {
            match key.to_string().as_str() {
                "name" => name = Some(value.parse()?),
                "path" => path = Some(value.parse()?),
                "registry" => registry = Some(value.parse()?),
                "config" => config = Some(value.parse()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown location key `{other}`; expected `name` and `path`, or \
                             `registry` with an optional `config`"
                        ),
                    ));
                }
            }
            Ok(())
        })?;

        match (name, path, registry, config) {
            (None, None, Some(registry), config) => Ok(Self::Registry { registry, config }),
            (Some(name), Some(path), None, None) => Ok(Self::Path { name, path }),
            (_, _, Some(_), _) => Err(syn::Error::new(
                span,
                "a `registry` location carries only `config` beside it; declare paths as their \
                 own `{ name, path }` entries",
            )),
            (_, _, None, Some(_)) => Err(syn::Error::new(
                span,
                "`config` is the wasm-pkg configuration of a `registry` location; a path \
                 location carries none",
            )),
            (name, _, None, None) => {
                let missing = if name.is_none() { "name" } else { "path" };
                Err(syn::Error::new(
                    span,
                    format!(
                        "location entry is missing `{missing}`; a location is `{{ name, path }}` \
                         or `{{ registry: ..., config: ... }}`"
                    ),
                ))
            }
        }
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
