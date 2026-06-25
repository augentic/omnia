use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, LitBool, LitStr, Path, Result, Token};

/// Configuration for the runtime macro.
///
/// Parses input in the form of 'host:backend' pairs. For example:
/// ```ignore
/// {
///     WasiHttp: HttpDefault,
///     WasiOtel: DefaultOtel,
///     ...
/// }
/// ```
pub struct Config {
    pub gen_main: bool,
    pub hosts: Vec<Host>,
    pub backends: Vec<Path>,
    pub embedded: Vec<(LitStr, Expr)>,
}

impl Parse for Config {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut main = false;
        let mut hosts = Hosts(Vec::new());
        let mut embedded = Vec::new();

        let settings;
        syn::braced!(settings in input);
        let settings = Punctuated::<Opt, Token![,]>::parse_terminated(&settings)?;

        for setting in settings.into_pairs() {
            match setting.into_value() {
                Opt::Main(m) => main = m,
                Opt::Host(h) => hosts = h,
                Opt::Embedded(e) => embedded = e,
            }
        }

        // deduplicate backends
        let mut backends = vec![];
        for host in &hosts.0 {
            if backends.iter().any(|b: &Path| b.get_ident() == host.backend.get_ident()) {
                continue;
            }
            backends.push(host.backend.clone());
        }

        Ok(Self {
            gen_main: main,
            hosts: hosts.0,
            backends,
            embedded,
        })
    }
}

mod kw {
    syn::custom_keyword!(main);
    syn::custom_keyword!(hosts);
    syn::custom_keyword!(embedded);
}

#[allow(clippy::large_enum_variant)]
enum Opt {
    Main(bool),
    Host(Hosts),
    Embedded(Vec<(LitStr, Expr)>),
}

impl Parse for Opt {
    fn parse(input: ParseStream) -> Result<Self> {
        let l = input.lookahead1();
        if l.peek(kw::main) {
            input.parse::<kw::main>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Main(input.parse::<LitBool>()?.value))
        } else if l.peek(kw::hosts) {
            input.parse::<kw::hosts>()?;
            input.parse::<Token![:]>()?;
            let list;
            syn::braced!(list in input);
            Ok(Self::Host(list.parse()?))
        } else if l.peek(kw::embedded) {
            input.parse::<kw::embedded>()?;
            input.parse::<Token![:]>()?;
            let list;
            syn::braced!(list in input);
            let entries = Punctuated::<EmbeddedEntry, Token![,]>::parse_terminated(&list)?;
            Ok(Self::Embedded(entries.into_iter().map(|e| (e.name, e.bytes)).collect()))
        } else {
            Err(l.error())
        }
    }
}

/// A single `"name" => <bytes-expr>` embedded-guest entry, where `<bytes-expr>`
/// is typically `include_bytes!("path/to/guest.wasm")`.
struct EmbeddedEntry {
    name: LitStr,
    bytes: Expr,
}

impl Parse for EmbeddedEntry {
    fn parse(input: ParseStream) -> Result<Self> {
        let name = input.parse::<LitStr>()?;
        input.parse::<Token![=>]>()?;
        let bytes = input.parse::<Expr>()?;
        Ok(Self { name, bytes })
    }
}

pub struct Hosts(Vec<Host>);

impl Parse for Hosts {
    fn parse(input: ParseStream) -> Result<Self> {
        let hosts = Punctuated::<Host, Token![,]>::parse_terminated(input)?;
        Ok(Self(hosts.into_iter().collect()))
    }
}

/// Information about a WASI host and its configuration.
pub struct Host {
    pub type_: Path,
    pub backend: Path,
}

impl Parse for Host {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let type_ = input.parse::<Path>()?;
        input.parse::<Token![:]>()?;
        let backend = input.parse::<Path>()?;
        Ok(Self { type_, backend })
    }
}
