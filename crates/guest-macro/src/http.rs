use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Error, Ident, LitStr, Path, Result, Token};

use crate::guest::{Config, handler_name};

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
    pub path: LitStr,
    pub params: Vec<Ident>,
    pub handler: Handler,
    pub function: Ident,
}

impl Parse for Route {
    fn parse(input: ParseStream) -> Result<Self> {
        let path: LitStr = input.parse()?;
        input.parse::<Token![:]>()?;

        let mut handler: Option<Handler> = None;

        let fields = Punctuated::<Opt, Token![|]>::parse_separated_nonempty(input)?;
        for field in fields.into_pairs() {
            match field.into_value() {
                Opt::Handler(h) => {
                    if handler.is_some() {
                        return Err(Error::new(h.method.span(), "cannot specify second handler"));
                    }
                    handler = Some(h);
                }
            }
        }

        // validate required fields
        let Some(handler) = handler else {
            return Err(Error::new(
                path.span(),
                "route is missing handler (e.g., `get(Request, Response)` or `post(Request, Response)`)",
            ));
        };

        // derived values
        let params = extract_params(&path);
        let function = handler_name(&path);

        Ok(Self {
            path,
            params,
            handler,
            function,
        })
    }
}

// Contains the HTTP method and the request and reply types.
pub struct Handler {
    method: Ident,
    request: Path,
    reply: Path,
    with_body: bool,
    with_query: bool,
}

// Parse the handler method in the form of `method(request, reply)`.
impl Parse for Handler {
    fn parse(input: ParseStream) -> Result<Self> {
        // parse method
        let method: Ident = input.parse()?;

        // parse request and reply
        let list;
        syn::parenthesized!(list in input);

        // ..request
        let request: Path = list.parse()?;

        // ..optional `with_body` or `with_query`
        let mut with_body = false;
        let mut with_query = false;

        let l = list.lookahead1();
        if l.peek(kw::with_body) {
            list.parse::<kw::with_body>()?;
            with_body = true;
        } else if l.peek(kw::with_query) {
            list.parse::<kw::with_query>()?;
            with_query = true;
        }

        // ..reply
        list.parse::<Token![,]>()?;
        let reply: Path = list.parse()?;

        // verify
        if method == "get" && with_body {
            return Err(Error::new(
                method.span(),
                "GET requests should not have a body; consider using query parameters",
            ));
        } else if method == "post" && with_query {
            return Err(Error::new(
                method.span(),
                "POST requests should not have query parameters; consider using body",
            ));
        }

        Ok(Self {
            method,
            request,
            reply,
            with_body,
            with_query,
        })
    }
}

mod kw {
    syn::custom_keyword!(get);
    syn::custom_keyword!(post);
    syn::custom_keyword!(with_query);
    syn::custom_keyword!(with_body);
}

enum Opt {
    Handler(Handler),
}

impl Parse for Opt {
    fn parse(input: ParseStream) -> Result<Self> {
        let l = input.lookahead1();
        if l.peek(kw::get) || l.peek(kw::post) {
            Ok(Self::Handler(input.parse::<Handler>()?))
        } else {
            Err(l.error())
        }
    }
}

fn extract_params(path: &LitStr) -> Vec<Ident> {
    path.value()
        .split('/')
        .filter(|s| s.starts_with('{') && s.ends_with('}'))
        .map(|s| &s[1..s.len() - 1])
        .map(|p| format_ident!("{p}"))
        .collect()
}

pub fn expand(http: &Http, config: &Config) -> TokenStream {
    let routes = http.routes.iter().map(expand_route);
    let handlers = http.routes.iter().map(|r| expand_handler(r, config));

    quote! {
        mod http {
            use omnia_sdk::api::{HttpResult, Reply};
            use omnia_sdk::{axum, omnia_wasi_http, omnia_wasi_otel, wasip3};
            use omnia_sdk::Handler;

            use super::*;

            pub struct Http;
            wasip3::http::proxy::export!(Http);

            impl wasip3::exports::http::handler::Guest for Http {
                #[omnia_wasi_otel::instrument]
                async fn handle(
                    request: wasip3::http::types::Request,
                ) -> Result<wasip3::http::types::Response, wasip3::http::types::ErrorCode> {
                    let router = axum::Router::new()
                        #(#routes)*;
                    omnia_wasi_http::serve(router, request).await
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
    let handler = &route.handler;
    let params = &route.params;
    let function = &route.function;
    let request = &handler.request;
    let reply = &handler.reply;
    let owner = &config.owner;
    let provider = &config.provider;

    let is_get = handler.method == "get";

    let args = if is_get {
        expand_get_args(params, handler.with_query)
    } else {
        expand_post_args(handler.with_body)
    };

    let input = if is_get {
        expand_get_input(params, handler.with_query)
    } else {
        expand_post_input(handler.with_body)
    };

    quote! {
        #[omnia_wasi_otel::instrument]
        async fn #function(#args) -> HttpResult<Reply<#reply>> {
            #request::handler(#input)?
                .provider(&#provider::new())
                .owner(#owner)
                .await
                .map_err(Into::into)
        }
    }
}

/// Builds the function arguments for a GET handler based on path parameters and query settings.
fn expand_get_args(params: &[Ident], with_query: bool) -> TokenStream {
    if params.is_empty() {
        if with_query {
            quote! { axum::extract::RawQuery(query): axum::extract::RawQuery }
        } else {
            quote! {}
        }
    } else if params.len() == 1 {
        quote! { axum::extract::Path(#(#params),*): axum::extract::Path<String> }
    } else {
        let param_types = vec![format_ident!("String"); params.len()];
        quote! { axum::extract::Path((#(#params),*)): axum::extract::Path<(#(#param_types),*)> }
    }
}

/// Builds the function arguments for a POST handler based on body settings.
fn expand_post_args(with_body: bool) -> TokenStream {
    if with_body {
        quote! { body: bytes::Bytes }
    } else {
        quote! {}
    }
}

/// Builds the input expression passed to the request handler for GET requests.
fn expand_get_input(params: &[Ident], with_query: bool) -> TokenStream {
    if params.is_empty() {
        if with_query {
            quote! { query }
        } else {
            quote! { () }
        }
    } else if params.len() == 1 {
        quote! { #(#params),* }
    } else {
        quote! { (#(#params),*) }
    }
}

/// Builds the input expression passed to the request handler for POST requests.
fn expand_post_input(with_body: bool) -> TokenStream {
    if with_body {
        quote! { body.to_vec() }
    } else {
        quote! { () }
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
