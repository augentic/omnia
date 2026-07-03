//! Implementation details for the `#[instrument]` attribute macro.

use quote::quote;
use syn::meta::ParseNestedMeta;
use syn::parse::Result;
use syn::{Expr, ItemFn, LitStr};

pub fn body(attrs: Attributes, item_fn: &ItemFn) -> proc_macro2::TokenStream {
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

#[cfg(test)]
mod tests {
    use super::{Attributes, body};

    #[test]
    fn async_fn_is_wrapped_in_instrument() {
        let item_fn: syn::ItemFn = syn::parse_quote! {
            async fn handler() { do_work().await }
        };
        let out = body(Attributes::default(), &item_fn).to_string();
        assert!(out.contains("Instrument"), "async body must use Instrument: {out}");
        assert!(out.contains("await"), "async body must be awaited: {out}");
    }

    #[test]
    fn sync_fn_uses_in_scope() {
        let item_fn: syn::ItemFn = syn::parse_quote! {
            fn handler() { do_work() }
        };
        let out = body(Attributes::default(), &item_fn).to_string();
        assert!(out.contains("in_scope"), "sync body must use in_scope: {out}");
        assert!(!out.contains("Instrument"), "sync body must not use Instrument: {out}");
    }

    #[test]
    fn defaults_span_name_to_fn_ident() {
        let item_fn: syn::ItemFn = syn::parse_quote! {
            fn my_handler() {}
        };
        let out = body(Attributes::default(), &item_fn).to_string();
        assert!(out.contains("my_handler"), "span name should default to the fn ident: {out}");
    }
}
