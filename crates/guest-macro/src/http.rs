use std::sync::LazyLock;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use regex::Regex;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Error, Ident, LitStr, Path, Result, Token};

use crate::guest::{Config, handler_name};

static PARAMS_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{([A-Za-z_][A-Za-z0-9_]*)\}").expect("should compile"));

pub struct Http {
    pub routes: Vec<Route>,
}

impl Parse for Http {
    fn parse(input: ParseStream) -> Result<Self> {
        let routes = Punctuated::<Route, Token![,]>::parse_terminated(input)?;
        Ok(Self {
            routes: routes.into_iter().collect(),
        })
    }
}

pub struct Route {
    path: LitStr,
    params: Vec<Ident>,
    handler: Handler,
    function: Ident,
    query: bool,
    body: bool,
}

impl Parse for Route {
    fn parse(input: ParseStream) -> Result<Self> {
        let path: LitStr = input.parse()?;
        input.parse::<Token![:]>()?;

        let mut handler: Option<Handler> = None;
        let mut query: Option<bool> = None;
        let mut body: Option<bool> = None;

        let fields = Punctuated::<Opt, Token![|]>::parse_separated_nonempty(input)?;

        for field in fields.into_pairs() {
            match field.into_value() {
                Opt::Handler(h) => {
                    if handler.is_some() {
                        return Err(Error::new(h.method.span(), "cannot specify second handler"));
                    }
                    handler = Some(h);
                }
                Opt::Query(q) => {
                    if query.is_some() {
                        return Err(Error::new(q.span(), "query is specified more than once"));
                    }
                    query = Some(q);
                }
                Opt::Body(b) => {
                    if body.is_some() {
                        return Err(Error::new(b.span(), "body is specified more than once"));
                    }
                    body = Some(b);
                }
            }
        }

        // validate required fields
        let Some(handler) = handler else {
            return Err(Error::new(path.span(), "route is missing `method`"));
        };

        // derived values
        let params = extract_params(&path);
        let function = handler_name(&path);

        Ok(Self {
            path,
            params,
            handler,
            function,
            query: query.unwrap_or_default(),
            body: body.unwrap_or_default(),
        })
    }
}

// Contains the HTTP method and the request and reply types.
struct Handler {
    method: Ident,
    request: Path,
    reply: Path,
}

// Parse the handler method in the form of `method(request, reply)`.
impl Parse for Handler {
    fn parse(input: ParseStream) -> Result<Self> {
        let method: Ident = input.parse()?;

        let list;
        syn::parenthesized!(list in input);

        let request: Path = list.parse()?;
        list.parse::<Token![,]>()?;
        let reply: Path = list.parse()?;

        Ok(Self {
            method,
            request,
            reply,
        })
    }
}

mod kw {
    syn::custom_keyword!(get);
    syn::custom_keyword!(post);
    syn::custom_keyword!(query);
    syn::custom_keyword!(body);
}

enum Opt {
    Handler(Handler),
    Query(bool),
    Body(bool),
}

impl Parse for Opt {
    fn parse(input: ParseStream) -> Result<Self> {
        let l = input.lookahead1();
        if l.peek(kw::get) || l.peek(kw::post) {
            Ok(Self::Handler(input.parse::<Handler>()?))
        } else if l.peek(kw::query) {
            input.parse::<kw::query>()?;
            Ok(Self::Query(true))
        } else if l.peek(kw::body) {
            input.parse::<kw::body>()?;
            Ok(Self::Body(true))
        } else {
            Err(l.error())
        }
    }
}

fn extract_params(path: &LitStr) -> Vec<Ident> {
    PARAMS_REGEX
        .captures_iter(&path.value())
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_owned()))
        .map(|p| format_ident!("{p}"))
        .collect()
}

pub fn expand(http: &Http, config: &Config) -> TokenStream {
    let routes = http.routes.iter().map(expand_route);
    let handlers = http.routes.iter().map(|r| expand_handler(r, config));

    quote! {
        mod http {
            use warp_sdk::api::{HttpResult, Reply};
            use warp_sdk::{axum, wasi_http, wasi_otel, wasip3};
            use warp_sdk::Handler;

            use super::*;

            pub struct Http;
            wasip3::http::proxy::export!(Http);

            impl wasip3::exports::http::handler::Guest for Http {
                #[wasi_otel::instrument]
                async fn handle(
                    request: wasip3::http::types::Request,
                ) -> Result<wasip3::http::types::Response, wasip3::http::types::ErrorCode> {
                    let router = axum::Router::new()
                        #(#routes)*;
                    wasi_http::serve(router, request).await
                }
            }

            #(#handlers)*
        }
    }
}

fn expand_route(route: &Route) -> TokenStream {
    let path = &route.path;
    let method = &route.handler.method;
    let function = &route.function;

    quote! {
        .route(#path, axum::routing::#method(#function))
    }
}

fn expand_handler(route: &Route, config: &Config) -> TokenStream {
    let function = &route.function;
    let method = &route.handler.method;
    let request = &route.handler.request;
    let reply = &route.handler.reply;
    let params = &route.params;
    let owner = &config.owner;
    let provider = &config.provider; // generate handler function name and signature

    let handler_fn = if method == "get" {
        let args = if params.is_empty() {
            if route.query {
                quote! { axum::extract::RawQuery(query): axum::extract::RawQuery }
            } else {
                quote! {}
            }
        } else if params.len() == 1 {
            quote! { axum::extract::Path(#(#params),*): axum::extract::Path<String> }
        } else {
            let mut param_types = Vec::new();
            for _ in 0..params.len() {
                param_types.push(format_ident!("String"));
            }
            quote! { axum::extract::Path((#(#params),*)): axum::extract::Path<(#(#param_types),*)> }
        };

        quote! { #function(#args) }
    } else {
        let args = if route.body {
            quote! { body: bytes::Bytes }
        } else {
            quote! {}
        };

        quote! { #function(#args) }
    };

    // generate request parameter and type
    let input = if method == "get" {
        if params.is_empty() {
            if route.query {
                quote! { query }
            } else {
                quote! { () }
            }
        } else if params.len() == 1 {
            quote! { #(#params),* }
        } else {
            quote! { (#(#params),*) }
        }
    } else {
        if route.body {
            quote! { body.to_vec() }
        } else {
            quote! { () }
        }
    };

    quote! {
        #[wasi_otel::instrument]
        async fn #handler_fn -> HttpResult<Reply<#reply>> {
            #request::handler(#input)?
                .provider(&#provider::new())
                .owner(#owner)
                .await
                .map_err(Into::into)
        }
    }
}

#[cfg(test)]
mod tests {
    use proc_macro2::Span;

    use super::*;

    #[test]
    fn test_parse_params() {
        let path = LitStr::new("{vehicle_id}/{trip_id}", Span::call_site());
        let params = extract_params(&path);
        assert_eq!(params, vec![format_ident!("vehicle_id"), format_ident!("trip_id")]);
    }
}
