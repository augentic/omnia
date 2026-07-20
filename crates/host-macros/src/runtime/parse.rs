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

/// Configuration for the runtime macro.
pub struct Config {
    pub mode: Mode,
    pub host_entries: Vec<HostEntry>,
    #[allow(clippy::struct_field_names)]
    pub config_file: Option<Expr>,
    pub manifest: ManifestSpec,
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

/// One `{ id: ..., source: ..., link: [...] }` guest entry.
pub struct GuestSpec {
    pub id: Expr,
    pub source: Expr,
    pub link: Vec<Expr>,
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
        let mut config_span: Option<Span> = None;
        let mut inline_span: Option<Span> = None;

        let settings;
        syn::braced!(settings in input);
        let settings = Punctuated::<Opt, Token![,]>::parse_terminated(&settings)?;

        for setting in settings.into_pairs() {
            match setting.into_value() {
                Opt::Mode(m) => mode = m,
                Opt::Hosts(h) => host_entries = h,
                Opt::Config(c, span) => {
                    config_file = Some(c);
                    config_span = Some(span);
                }
                Opt::Guests(g, span) => {
                    manifest.guests = g;
                    inline_span.get_or_insert(span);
                }
                Opt::Mounts(m, span) => {
                    manifest.mounts = m;
                    inline_span.get_or_insert(span);
                }
                Opt::Link(l, span) => {
                    manifest.link = l;
                    inline_span.get_or_insert(span);
                }
                Opt::Routes(r, span) => {
                    manifest.routes = r;
                    inline_span.get_or_insert(span);
                }
            }
        }

        if let (Some(_), Some(inline)) = (config_span, inline_span) {
            return Err(syn::Error::new(
                inline,
                "`config:` and inline manifest keys (`guests`, `mounts`, `link`, `routes`) are \
                 mutually exclusive",
            ));
        }

        Ok(Self {
            mode,
            host_entries,
            config_file,
            manifest,
        })
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
}

enum Opt {
    Mode(Mode),
    Hosts(Vec<HostEntry>),
    Config(Expr, Span),
    Guests(Vec<GuestSpec>, Span),
    Mounts(Vec<MountSpec>, Span),
    Link(Vec<Expr>, Span),
    Routes(RoutesSpec, Span),
}

impl Parse for Opt {
    fn parse(input: ParseStream) -> Result<Self> {
        let l = input.lookahead1();
        if l.peek(kw::mode) {
            input.parse::<kw::mode>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Mode(parse_mode(input)?))
        } else if l.peek(kw::hosts) {
            input.parse::<kw::hosts>()?;
            input.parse::<Token![:]>()?;
            let list;
            syn::braced!(list in input);
            Ok(Self::Hosts(parse_host_entries(&list)?))
        } else if l.peek(kw::config) {
            let key = input.parse::<kw::config>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Config(input.parse()?, key.span))
        } else if l.peek(kw::guests) {
            let key = input.parse::<kw::guests>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Guests(parse_bracketed_list(input)?, key.span))
        } else if l.peek(kw::mounts) {
            let key = input.parse::<kw::mounts>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Mounts(parse_bracketed_list(input)?, key.span))
        } else if l.peek(kw::link) {
            let key = input.parse::<kw::link>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Link(parse_expr_list(input)?, key.span))
        } else if l.peek(kw::routes) {
            let key = input.parse::<kw::routes>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Routes(input.parse()?, key.span))
        } else {
            Err(l.error())
        }
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

/// Parse `[ expr, expr, ... ]`.
fn parse_expr_list(input: ParseStream) -> Result<Vec<Expr>> {
    parse_bracketed_list::<Expr>(input)
}

impl Parse for GuestSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let content;
        let brace = syn::braced!(content in input);
        let mut id = None;
        let mut source = None;
        let mut link = Vec::new();

        while !content.is_empty() {
            let key: Ident = content.parse()?;
            content.parse::<Token![:]>()?;
            match key.to_string().as_str() {
                "id" => id = Some(content.parse()?),
                "source" => source = Some(content.parse()?),
                "link" => link = parse_expr_list(&content)?,
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown guest key `{other}`; expected `id`, `source`, or `link`"),
                    ));
                }
            }
            if !content.is_empty() {
                content.parse::<Token![,]>()?;
            }
        }

        let missing =
            |key| syn::Error::new(brace.span.join(), format!("guest entry is missing `{key}`"));
        Ok(Self {
            id: id.ok_or_else(|| missing("id"))?,
            source: source.ok_or_else(|| missing("source"))?,
            link,
        })
    }
}

impl Parse for MountSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let content;
        let brace = syn::braced!(content in input);
        let mut name = None;
        let mut path = None;
        let mut writable = None;

        while !content.is_empty() {
            let key: Ident = content.parse()?;
            content.parse::<Token![:]>()?;
            match key.to_string().as_str() {
                "name" => name = Some(content.parse()?),
                "path" => path = Some(content.parse()?),
                "writable" => writable = Some(content.parse()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown mount key `{other}`; expected `name`, `path`, or `writable`"
                        ),
                    ));
                }
            }
            if !content.is_empty() {
                content.parse::<Token![,]>()?;
            }
        }

        let missing =
            |key| syn::Error::new(brace.span.join(), format!("mount entry is missing `{key}`"));
        Ok(Self {
            name: name.ok_or_else(|| missing("name"))?,
            path: path.ok_or_else(|| missing("path"))?,
            writable,
        })
    }
}

impl Parse for RoutesSpec {
    fn parse(input: ParseStream) -> Result<Self> {
        let content;
        syn::braced!(content in input);
        let mut routes = Self::default();

        while !content.is_empty() {
            let key: Ident = content.parse()?;
            content.parse::<Token![:]>()?;
            match key.to_string().as_str() {
                "http" => routes.http = parse_route_entries(&content, "prefix")?,
                "messaging" => routes.messaging = parse_route_entries(&content, "topic")?,
                "websocket" => routes.websocket = parse_route_entries(&content, "route")?,
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
            if !content.is_empty() {
                content.parse::<Token![,]>()?;
            }
        }

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
        let content;
        let brace = syn::braced!(content in list);
        let mut key = None;
        let mut guest = None;

        while !content.is_empty() {
            let field: Ident = content.parse()?;
            content.parse::<Token![:]>()?;
            match field.to_string().as_str() {
                k if k == match_key => key = Some(content.parse()?),
                "guest" => guest = Some(content.parse()?),
                other => {
                    return Err(syn::Error::new(
                        field.span(),
                        format!("unknown route key `{other}`; expected `{match_key}` or `guest`"),
                    ));
                }
            }
            if !content.is_empty() {
                content.parse::<Token![,]>()?;
            }
        }

        let missing =
            |key| syn::Error::new(brace.span.join(), format!("route entry is missing `{key}`"));
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
