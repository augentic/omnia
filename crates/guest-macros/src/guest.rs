use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Error, Ident, LitStr, Result, Token};

use crate::command::{self, Command};
use crate::http::{self, Http};
use crate::messaging::{self, Messaging};

pub struct Config {
    pub owner: LitStr,
    pub provider: Ident,
    pub http: Option<Http>,
    pub messaging: Option<Messaging>,
    pub command: Option<Command>,
}

impl Parse for Config {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut owner: Option<LitStr> = None;
        let mut provider: Option<Ident> = None;
        let mut http: Option<Http> = None;
        let mut messaging: Option<Messaging> = None;
        let mut command: Option<Command> = None;

        let settings;
        let brace = syn::braced!(settings in input);
        let settings = Punctuated::<Opt, Token![,]>::parse_terminated(&settings)?;

        for setting in settings.into_pairs() {
            match setting.into_value() {
                Opt::Owner(o) => {
                    if owner.is_some() {
                        return Err(Error::new(o.span(), "cannot specify second owner"));
                    }
                    owner = Some(o);
                }
                Opt::Provider(p) => {
                    if provider.is_some() {
                        return Err(Error::new(p.span(), "cannot specify second provider"));
                    }
                    provider = Some(p);
                }
                Opt::Http(h) => {
                    http = Some(h);
                }
                Opt::Messaging(m) => {
                    messaging = Some(m);
                }
                Opt::Command(c) => {
                    command = Some(c);
                }
            }
        }

        // Point missing-field errors at the config braces, not the macro name.
        let Some(owner) = owner else {
            return Err(Error::new(brace.span.join(), "missing `owner`"));
        };
        let Some(provider) = provider else {
            return Err(Error::new(brace.span.join(), "missing `provider`"));
        };

        Ok(Self {
            owner,
            provider,
            http,
            messaging,
            command,
        })
    }
}

mod kw {
    syn::custom_keyword!(owner);
    syn::custom_keyword!(provider);
    syn::custom_keyword!(http);
    syn::custom_keyword!(messaging);
    syn::custom_keyword!(command);
}

enum Opt {
    Owner(syn::LitStr),
    Provider(Ident),
    Http(Http),
    Messaging(Messaging),
    Command(Command),
}

impl Parse for Opt {
    fn parse(input: ParseStream) -> Result<Self> {
        let l = input.lookahead1();
        if l.peek(kw::owner) {
            input.parse::<kw::owner>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Owner(input.parse::<LitStr>()?))
        } else if l.peek(kw::provider) {
            input.parse::<kw::provider>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Provider(input.parse::<Ident>()?))
        } else if l.peek(kw::http) {
            input.parse::<kw::http>()?;
            input.parse::<Token![:]>()?;
            let list;
            syn::bracketed!(list in input);
            Ok(Self::Http(list.parse()?))
        } else if l.peek(kw::messaging) {
            input.parse::<kw::messaging>()?;
            input.parse::<Token![:]>()?;
            let list;
            syn::bracketed!(list in input);
            Ok(Self::Messaging(list.parse()?))
        } else if l.peek(kw::command) {
            input.parse::<kw::command>()?;
            input.parse::<Token![:]>()?;
            Ok(Self::Command(input.parse()?))
        } else {
            Err(l.error())
        }
    }
}

pub fn expand(config: &Config) -> TokenStream {
    let http_mod = config.http.as_ref().map(|h| http::expand(h, config));
    let messaging_mod = config
        .messaging
        .as_ref()
        .map(|m| messaging::expand(m, config))
        .map(|body| quote! { #[cfg(target_arch = "wasm32")] #body });
    let command_mod = config.command.as_ref().map(command::expand);
    let http_export = config.http.as_ref().map(|_| {
        quote! {
            #[doc(hidden)]
            pub use __buildgen_guest::http::router as http_router;
        }
    });

    quote! {
        mod __buildgen_guest {
            #[allow(unused_imports, reason = "generated glob for user-declared types")]
            use super::*;

            #http_mod
            #messaging_mod
            #command_mod
        }
        #http_export
    }
}

// Derive a handler method name from an HTTP path or messaging topic,
// pointing at the offending literal when the result is not a valid
// identifier (e.g. a topic starting with a digit).
pub fn handler_name(path: &LitStr) -> Result<Ident> {
    let path_str = path.value();
    let name = path_str
        .trim_start_matches('/')
        .replace(['/', '-', '.'], "_")
        .replace(['{', '}'], "")
        .to_lowercase();
    syn::parse_str::<Ident>(&name).map_err(|_parse_err| {
        Error::new(
            path.span(),
            format!(
                "cannot derive a handler name from `{path_str}`: `{name}` is not a valid identifier"
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use proc_macro2::Span;
    use quote::{format_ident, quote};

    use super::*;

    #[test]
    fn method_from_path() {
        // simple path
        let path = LitStr::new("/inbound/xml", Span::call_site());
        let name = handler_name(&path).expect("valid identifier");
        assert_eq!(name, format_ident!("inbound_xml"));

        // path parameters
        let path = LitStr::new("/set-trip/{vehicle_id}/{trip_id}", Span::call_site());
        let name = handler_name(&path).expect("valid identifier");
        assert_eq!(name, format_ident!("set_trip_vehicle_id_trip_id"));

        // path with dots
        let path = LitStr::new("/some/path/data.json", Span::call_site());
        let name = handler_name(&path).expect("valid identifier");
        assert_eq!(name, format_ident!("some_path_data_json"));
    }

    #[test]
    fn method_from_invalid_path() {
        let path = LitStr::new("9-lives.v1", Span::call_site());
        let err = handler_name(&path).expect_err("digit-leading name is rejected");
        assert!(err.to_string().contains("not a valid identifier"));
    }

    #[test]
    fn parse_config() {
        let input = quote!({
            owner: "at",
            provider: MyProvider,
            http: [
                "/jobs/detector": get(DetectionRequest with_query, DetectionReply)
            ],
            messaging: [
                "realtime-r9k.v1": R9kMessage,
            ]
        });

        let parsed: Config = syn::parse2(input).expect("should parse");

        let http = parsed.http.expect("should have http");
        assert_eq!(http.routes.len(), 1);
        assert_eq!(http.routes[0].path.value(), "/jobs/detector");

        let messaging = parsed.messaging.expect("should have messaging");
        assert_eq!(messaging.topics.len(), 1);
        assert_eq!(messaging.topics[0].pattern.value(), "realtime-r9k.v1");
    }

    #[test]
    fn parse_http_path_params() {
        let input = quote!({
            owner: "at",
            provider: MyProvider,
            http: [
                "/path/params/{vehicle_id}/{trip_id}": get(SetTripRequest, SetTripReply),
            ]
        });

        let parsed: Config = syn::parse2(input).expect("should parse");
        let http = parsed.http.expect("should have http");

        assert_eq!(http.routes.len(), 1);
        assert_eq!(http.routes[0].path.value(), "/path/params/{vehicle_id}/{trip_id}");
    }

    #[test]
    fn parse_command() {
        let input = quote!({
            owner: "at",
            provider: MyProvider,
            command: dispatch,
        });

        let parsed: Config = syn::parse2(input).expect("should parse");
        let command = parsed.command.expect("should have command");
        let dispatch = &command.dispatch;
        assert_eq!(quote!(#dispatch).to_string(), "dispatch");
    }
}
