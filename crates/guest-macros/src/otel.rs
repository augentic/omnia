//! Implementation details for the `#[instrument]` attribute macro.

use quote::quote;
use syn::meta::ParseNestedMeta;
use syn::parse::Result;
use syn::{Expr, ItemFn, LitStr};

pub fn body(attrs: Attributes, item_fn: &ItemFn) -> proc_macro2::TokenStream {
    let name = item_fn.sig.ident.clone();
    let block = item_fn.block.clone();

    let span_name = attrs.name.unwrap_or_else(|| LitStr::new(&name.to_string(), name.span()));
    // Routed through `omnia_wasi_otel` so callers need no direct `tracing` dependency.
    let tracing = quote! { omnia_wasi_otel::__private::tracing };
    let level =
        attrs.level.map_or_else(|| quote! { #tracing::Level::INFO }, |level| quote! {#level});

    // The closure defers `span!` until `scope` has initialized telemetry.
    if item_fn.sig.asyncness.is_some() {
        quote! {
            omnia_wasi_otel::scope(move || #tracing::Instrument::instrument(
                async move #block,
                #tracing::span!(#level, #span_name)
            )).await
        }
    } else {
        // a sync function is never an export: it only opens a span
        quote! {
            #tracing::span!(#level, #span_name).in_scope(|| {
                #block
            })
        }
    }
}

#[derive(Default)]
pub struct Attributes {
    name: Option<LitStr>,
    level: Option<Expr>,
}

// See https://docs.rs/syn/latest/syn/meta/fn.parser.html
impl Attributes {
    pub fn parse(&mut self, meta: &ParseNestedMeta) -> Result<()> {
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
