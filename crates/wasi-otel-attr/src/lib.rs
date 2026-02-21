#![doc = include_str!("../README.md")]

//! # OpenTelemetry Attribute Macros

#![forbid(unsafe_code)]

use proc_macro::TokenStream;
use quote::quote;
use syn::meta::{self, ParseNestedMeta};
use syn::parse::Result;
use syn::{Expr, ItemFn, LitStr, parse_macro_input};

/// Instruments a function using the `[wasi_otel::instrument]` function.
///
/// This macro can be used to automatically create spans for functions, making
/// it easier to add observability to your code.
#[proc_macro_attribute]
pub fn instrument(args: TokenStream, item: TokenStream) -> TokenStream {
    // macro's attributes
    let mut attrs = Attributes::default();
    let arg_parser = meta::parser(|meta| attrs.parse(&meta));
    parse_macro_input!(args with arg_parser);

    let item_fn = parse_macro_input!(item as ItemFn);
    let signature = &item_fn.sig;
    let body = body(attrs, &item_fn);

    // recreate function with the instrument macro wrapping the body
    let new_fn = quote! {
        #signature {
            let _guard = ::omnia_wasi_otel::init();
            #body
        }
    };

    TokenStream::from(new_fn)
}

fn body(attrs: Attributes, item_fn: &ItemFn) -> proc_macro2::TokenStream {
    let name = item_fn.sig.ident.clone();
    let block = item_fn.block.clone();

    let span_name = attrs.name.unwrap_or_else(|| LitStr::new(&name.to_string(), name.span()));
    let level =
        attrs.level.map_or_else(|| quote! { ::tracing::Level::INFO }, |level| quote! {#level});

    // `instrument` async functions
    if item_fn.sig.asyncness.is_some() {
        quote! {
            ::tracing::Instrument::instrument(
                async move #block,
                ::tracing::span!(#level, #span_name)
            ).await
        }
    } else {
        quote! {
            ::tracing::span!(#level, #span_name).in_scope(|| {
                #block
            })
        }
    }
}

#[derive(Default)]
struct Attributes {
    name: Option<LitStr>,
    level: Option<Expr>,
}

// See https://docs.rs/syn/latest/syn/meta/fn.parser.html
impl Attributes {
    fn parse(&mut self, meta: &ParseNestedMeta) -> Result<()> {
        if meta.path.is_ident("name") {
            self.name = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("level") {
            self.level = Some(meta.value()?.parse()?);
        } else {
            return Err(meta.error("unsupported property"));
        }

        Ok(())
    }
}
