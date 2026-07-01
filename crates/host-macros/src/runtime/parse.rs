//! # Parse
//!
//! Parses the runtime macro token stream input into structured values.

use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{LitBool, Path, Result, Token};

/// Configuration for the runtime macro.
pub struct Config {
    pub command: bool,
    pub host_entries: Vec<HostEntry>,
}

/// One `Host: Backend` wiring from the `hosts: { ... }` block.
pub struct HostEntry {
    pub host: Path,
    pub backend: Path,
}

impl Parse for Config {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut command = false;
        let mut host_entries = Vec::new();

        let settings;
        syn::braced!(settings in input);
        let settings = Punctuated::<Opt, Token![,]>::parse_terminated(&settings)?;

        for setting in settings.into_pairs() {
            match setting.into_value() {
                Opt::Command(c) => command = c,
                Opt::Hosts(h) => host_entries = h,
            }
        }

        Ok(Self { command, host_entries })
    }
}

mod kw {
    syn::custom_keyword!(command);
    syn::custom_keyword!(hosts);
}

#[allow(clippy::large_enum_variant)]
enum Opt {
    Command(bool),
    Hosts(Vec<HostEntry>),
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
            Ok(Self::Hosts(parse_host_entries(&list)?))
        } else {
            Err(l.error())
        }
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
    Ok(Punctuated::<HostEntry, Token![,]>::parse_terminated(input)?
        .into_iter()
        .collect())
}
