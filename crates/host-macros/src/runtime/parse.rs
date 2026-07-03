//! # Parse
//!
//! Parses the runtime macro token stream input into structured values.

use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, Path, Result, Token};

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
}

/// One `Host: Backend` wiring from the `hosts: { ... }` block.
pub struct HostEntry {
    pub host: Path,
    pub backend: Path,
}

impl Parse for Config {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut mode = Mode::default();
        let mut host_entries = Vec::new();

        let settings;
        syn::braced!(settings in input);
        let settings = Punctuated::<Opt, Token![,]>::parse_terminated(&settings)?;

        for setting in settings.into_pairs() {
            match setting.into_value() {
                Opt::Mode(m) => mode = m,
                Opt::Hosts(h) => host_entries = h,
            }
        }

        Ok(Self { mode, host_entries })
    }
}

mod kw {
    syn::custom_keyword!(mode);
    syn::custom_keyword!(hosts);
}

enum Opt {
    Mode(Mode),
    Hosts(Vec<HostEntry>),
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
