//! # Parse
//!
//! Parses the runtime macro token stream input into structured values.

use proc_macro2::Span;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
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
    pub resolver: Option<Expr>,
    pub http_paths: Option<Expr>,
    pub http_listener: Option<Expr>,
}

/// One `Host: Backend` wiring from the `hosts: { ... }` block.
pub struct HostEntry {
    pub host: Path,
    pub backend: Path,
}

/// Inline manifest keys (`guests`, `mounts`, `link`, `routes`) parsed from
/// `runtime!({ ... })`; mirrors the `omnia::Manifest` schema.
#[derive(Default)]
pub struct ManifestSpec {
    pub guests: Vec<GuestSpec>,
    pub mounts: Vec<MountSpec>,
    pub link: Vec<Expr>,
    pub routes: RoutesSpec,
}

impl ManifestSpec {
    pub const fn is_empty(&self) -> bool {
        self.guests.is_empty()
            && self.mounts.is_empty()
            && self.link.is_empty()
            && self.routes.is_empty()
    }
}

/// One `{ id: ..., source: ..., link: [...], command: true }` guest entry.
pub struct GuestSpec {
    pub id: Expr,
    pub source: Expr,
    pub link: Vec<Expr>,
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

/// Per-trigger route lists from the `routes: { ... }` block.
#[derive(Default)]
pub struct RoutesSpec {
    pub http: Vec<RouteEntry>,
    pub messaging: Vec<RouteEntry>,
    pub websocket: Vec<RouteEntry>,
}

impl RoutesSpec {
    const fn is_empty(&self) -> bool {
        self.http.is_empty() && self.messaging.is_empty() && self.websocket.is_empty()
    }
}

/// One route: a match key (`prefix`/`topic`/`route`) mapped to a target guest.
pub struct RouteEntry {
    pub key: Expr,
    pub guest: Expr,
}

impl Parse for Config {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut mode = Mode::default();
        let mut host_entries = Vec::new();
        let mut config_file = None;
        let mut manifest = ManifestSpec::default();
        let mut resolver = None;
        let mut http_paths = None;
        let mut http_listener = None;
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
                OptValue::Guests(g) => {
                    manifest.guests = g;
                    inline_span.get_or_insert(span);
                }
                OptValue::Mounts(m) => {
                    manifest.mounts = m;
                    inline_span.get_or_insert(span);
                }
                OptValue::Link(l) => {
                    manifest.link = l;
                    inline_span.get_or_insert(span);
                }
                OptValue::Routes(r) => {
                    manifest.routes = r;
                    inline_span.get_or_insert(span);
                }
                OptValue::Resolver(r) => resolver = Some(r),
                OptValue::HttpPaths(p) => http_paths = Some(p),
                OptValue::HttpListener(l) => http_listener = Some(l),
            }
        }

        let config = Self {
            mode,
            host_entries,
            config_file,
            manifest,
            resolver,
            http_paths,
            http_listener,
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
                "`config:` and inline manifest keys (`guests`, `mounts`, `link`, `routes`) are \
                 mutually exclusive",
            ));
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
    syn::custom_keyword!(guests);
    syn::custom_keyword!(mounts);
    syn::custom_keyword!(link);
    syn::custom_keyword!(routes);
    syn::custom_keyword!(resolver);
    syn::custom_keyword!(http_paths);
    syn::custom_keyword!(http_listener);
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
    Guests(Vec<GuestSpec>),
    Mounts(Vec<MountSpec>),
    Link(Vec<Expr>),
    Routes(RoutesSpec),
    Resolver(Expr),
    HttpPaths(Expr),
    HttpListener(Expr),
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
        } else if l.peek(kw::guests) {
            let key = input.parse::<kw::guests>()?;
            input.parse::<Token![:]>()?;
            ("guests", key.span, OptValue::Guests(parse_bracketed_list(input)?))
        } else if l.peek(kw::mounts) {
            let key = input.parse::<kw::mounts>()?;
            input.parse::<Token![:]>()?;
            ("mounts", key.span, OptValue::Mounts(parse_bracketed_list(input)?))
        } else if l.peek(kw::link) {
            let key = input.parse::<kw::link>()?;
            input.parse::<Token![:]>()?;
            ("link", key.span, OptValue::Link(parse_bracketed_list(input)?))
        } else if l.peek(kw::routes) {
            let key = input.parse::<kw::routes>()?;
            input.parse::<Token![:]>()?;
            ("routes", key.span, OptValue::Routes(input.parse()?))
        } else if l.peek(kw::resolver) {
            let key = input.parse::<kw::resolver>()?;
            input.parse::<Token![:]>()?;
            ("resolver", key.span, OptValue::Resolver(input.parse()?))
        } else if l.peek(kw::http_paths) {
            let key = input.parse::<kw::http_paths>()?;
            input.parse::<Token![:]>()?;
            ("http_paths", key.span, OptValue::HttpPaths(input.parse()?))
        } else if l.peek(kw::http_listener) {
            let key = input.parse::<kw::http_listener>()?;
            input.parse::<Token![:]>()?;
            ("http_listener", key.span, OptValue::HttpListener(input.parse()?))
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
        Ok(Self { host, backend })
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
/// stream positioned at its value) to `field`. Returns the brace span for
/// missing-key diagnostics.
fn parse_kv_block(
    input: ParseStream, mut field: impl FnMut(&Ident, ParseStream) -> Result<()>,
) -> Result<Span> {
    let content;
    let brace = syn::braced!(content in input);
    while !content.is_empty() {
        let key: Ident = content.parse()?;
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
        let mut link = Vec::new();
        let mut command = false;
        let mut command_span = None;

        let span = parse_kv_block(input, |key, value| {
            match key.to_string().as_str() {
                "id" => id = Some(value.parse()?),
                "source" => source = Some(value.parse()?),
                "link" => link = parse_bracketed_list(value)?,
                "command" => {
                    let lit: syn::LitBool = value.parse()?;
                    command = lit.value();
                    command_span = command.then(|| key.span());
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown guest key `{other}`; expected `id`, `source`, `link`, or \
                             `command`"
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
            link,
            command,
            command_span,
        })
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

impl Parse for RoutesSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut routes = Self::default();

        parse_kv_block(input, |key, value| {
            match key.to_string().as_str() {
                "http" => routes.http = parse_route_entries(value, "prefix")?,
                "messaging" => routes.messaging = parse_route_entries(value, "topic")?,
                "websocket" => routes.websocket = parse_route_entries(value, "route")?,
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

/// Parse `[ { <match_key>: ..., guest: ... }, ... ]` route entries; the match
/// key is `prefix` (http), `topic` (messaging), or `route` (websocket).
fn parse_route_entries(input: ParseStream, match_key: &str) -> Result<Vec<RouteEntry>> {
    let list;
    syn::bracketed!(list in input);
    let mut entries = Vec::new();

    while !list.is_empty() {
        let mut key = None;
        let mut guest = None;

        let span = parse_kv_block(&list, |field, value| {
            match field.to_string().as_str() {
                k if k == match_key => key = Some(value.parse()?),
                "guest" => guest = Some(value.parse()?),
                other => {
                    return Err(syn::Error::new(
                        field.span(),
                        format!("unknown route key `{other}`; expected `{match_key}` or `guest`"),
                    ));
                }
            }
            Ok(())
        })?;

        let missing = |key| syn::Error::new(span, format!("route entry is missing `{key}`"));
        entries.push(RouteEntry {
            key: key.ok_or_else(|| missing(match_key))?,
            guest: guest.ok_or_else(|| missing("guest"))?,
        });

        if !list.is_empty() {
            list.parse::<Token![,]>()?;
        }
    }

    Ok(entries)
}
