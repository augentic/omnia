use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Error, Ident, LitStr, Path, Result, Token};

use crate::guest::Config;

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
    pub handler: Handler,
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

        Ok(Self { path, handler })
    }
}

// Contains the HTTP method and the request type. The reply type and the
// legacy `with_body` / `with_query` markers still parse (grammar
// compatibility) but extraction is uniformly typed: path parameters, query
// pairs, and the JSON body merge into `Handler::Input` via serde, so the
// route constructor needs only the request type.
pub struct Handler {
    method: Ident,
    request: Path,
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

        // ..reply (parsed for grammar compatibility; the route constructor
        // derives the reply type from the Handler impl)
        list.parse::<Token![,]>()?;
        let _reply: Path = list.parse()?;

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

        Ok(Self { method, request })
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

pub fn expand(http: &Http, config: &Config) -> TokenStream {
    let provider = &config.provider;
    let owner = &config.owner;
    let routes = http.routes.iter().map(|r| expand_route(r, config));

    quote! {
        // `pub` so the parent module's `http_router` re-export can name it;
        // `__buildgen_guest` itself stays private, so nothing escapes the
        // invoking module.
        pub mod http {
            use super::*;

            /// Target-neutral router over the declared route table. The
            /// owner and provider arrive as router state via the
            /// [`omnia_guest::api::Client`], so the same router serves the
            /// wasm guest export and a native listener.
            pub fn router(
                client: omnia_guest::api::Client<#provider>,
            ) -> omnia_guest::axum::Router {
                omnia_guest::axum::Router::new()
                    #(#routes)*
                    .with_state(client)
            }

            #[cfg(target_arch = "wasm32")]
            mod wasm {
                use omnia_guest::{omnia_wasi_http, omnia_wasi_otel, wasip3};

                use super::*;

                pub struct Http;
                wasip3::http::proxy::export!(Http);

                // Build the route table once; `axum::Router` is cheap to
                // clone (internally reference-counted) so each request
                // reuses it rather than rebuilding the whole graph.
                static ROUTER: std::sync::LazyLock<omnia_guest::axum::Router> =
                    std::sync::LazyLock::new(|| {
                        router(
                            omnia_guest::api::Client::new(#owner).provider(
                                <#provider as omnia_guest::api::DefaultProvider>::new(),
                            ),
                        )
                    });

                impl wasip3::exports::http::handler::Guest for Http {
                    #[omnia_wasi_otel::instrument]
                    async fn handle(
                        request: wasip3::http::types::Request,
                    ) -> Result<wasip3::http::types::Response, wasip3::http::types::ErrorCode> {
                        omnia_wasi_http::serve(ROUTER.clone(), request).await
                    }
                }
            }
        }
    }
}

fn expand_route(route: &Route, config: &Config) -> TokenStream {
    let path = &route.path;
    let method = &route.handler.method;
    let request = &route.handler.request;
    let provider = &config.provider;

    quote! {
        .route(#path, omnia_guest::api::route::#method::<#request, #provider>())
    }
}
