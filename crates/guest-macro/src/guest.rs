use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Error, Ident, LitStr, Result, Token};

use crate::http::{self, Http};
use crate::messaging::{self, Messaging};

pub struct Config {
    pub owner: LitStr,
    pub provider: Ident,
    pub http: Option<Http>,
    pub messaging: Option<Messaging>,
}

impl Parse for Config {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut owner: Option<LitStr> = None;
        let mut provider: Option<Ident> = None;
        let mut http: Option<Http> = None;
        let mut messaging: Option<Messaging> = None;

        let settings;
        syn::braced!(settings in input);
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
            }
        }

        let Some(owner) = owner else {
            return Err(Error::new(Span::call_site(), "missing `owner`"));
        };
        let Some(provider) = provider else {
            return Err(Error::new(Span::call_site(), "missing `provider`"));
        };

        Ok(Self {
            owner,
            provider,
            http,
            messaging,
        })
    }
}

mod kw {
    syn::custom_keyword!(owner);
    syn::custom_keyword!(provider);
    syn::custom_keyword!(http);
    syn::custom_keyword!(messaging);
}

#[allow(clippy::large_enum_variant)]
enum Opt {
    Owner(syn::LitStr),
    Provider(Ident),
    Http(Http),
    Messaging(Messaging),
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
        } else {
            Err(l.error())
        }
    }
}

pub fn expand(config: &Config) -> TokenStream {
    let http_mod = config.http.as_ref().map(|h| http::expand(h, config));
    let messaging_mod = config.messaging.as_ref().map(|m| messaging::expand(m, config));

    quote! {
        #[cfg(target_arch = "wasm32")]
        mod __buildgen_guest {
            use yetti_sdk::anyhow::{Context, Result};
            use yetti_sdk::api::Client;

            use super::*;

            #http_mod
            #messaging_mod
        }
    }
}

// Derive a handler method name from an HTTP path or messaging topic.
pub fn handler_name(path: &LitStr) -> Ident {
    let path_str = path.value();
    let name = path_str
        .trim_start_matches('/')
        .replace(['/', '-', '.'], "_")
        .replace(['{', '}'], "")
        .to_lowercase();
    format_ident!("{name}")
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::*;

    #[test]
    fn method_from_path() {
        // simple path
        let path = LitStr::new("/inbound/xml", Span::call_site());
        let name = handler_name(&path);
        assert_eq!(name, format_ident!("inbound_xml"));

        // path parameters
        let path = LitStr::new("/set-trip/{vehicle_id}/{trip_id}", Span::call_site());
        let name = handler_name(&path);
        assert_eq!(name, format_ident!("set_trip_vehicle_id_trip_id"));

        // path with dots
        let path = LitStr::new("/some/path/data.json", Span::call_site());
        let name = handler_name(&path);
        assert_eq!(name, format_ident!("some_path_data_json"));
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
        assert!(http.routes[0].params.is_empty());

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
        assert_eq!(http.routes[0].params.len(), 2);
        assert_eq!(http.routes[0].params[0].to_string(), "vehicle_id");
        assert_eq!(http.routes[0].params[1].to_string(), "trip_id");
    }
}
