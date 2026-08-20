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
}

/// One `Host: Backend` wiring from the `hosts: { ... }` block.
pub struct HostEntry {
    pub host: Path,
    pub backend: Path,
}

/// Inline manifest keys (`guests`, `mounts`) parsed from `runtime!({ ... })`;
/// mirrors the `omnia::Manifest` schema.
#[derive(Default)]
pub struct ManifestSpec {
    pub guests: Vec<GuestSpec>,
    pub mounts: Vec<MountSpec>,
}

impl ManifestSpec {
    pub const fn is_empty(&self) -> bool {
        self.guests.is_empty() && self.mounts.is_empty()
    }
}

/// One `{ id: ..., source: ..., link: [...], routes: { ... }, command: true }`
/// guest entry.
pub struct GuestSpec {
    pub id: Expr,
    pub source: Expr,
    pub link: Vec<Expr>,
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
            }
        }

        let config = Self {
            mode,
            host_entries,
            config_file,
            manifest,
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
                "`config:` and inline manifest keys (`guests`, `mounts`) are mutually exclusive",
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
    syn::custom_keyword!(routes);
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
        } else if input.peek(kw::routes) {
            // A pointed migration diagnostic, deliberately outside the
            // lookahead set so unrelated unknown keys don't suggest `routes`.
            let key = input.parse::<kw::routes>()?;
            return Err(syn::Error::new(
                key.span,
                "the top-level `routes:` key was removed; declare routes on each guest entry \
                 (`guests: [{ id: ..., source: ..., routes: { http: [...] } }]`)",
            ));
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
        let mut routes = GuestRoutesSpec::default();
        let mut command = false;
        let mut command_span = None;

        let span = parse_kv_block(input, |key, value| {
            match key.to_string().as_str() {
                "id" => id = Some(value.parse()?),
                "source" => source = Some(value.parse()?),
                "link" => link = parse_bracketed_list(value)?,
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
                            "unknown guest key `{other}`; expected `id`, `source`, `link`, \
                             `routes`, or `command`"
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
            routes,
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
