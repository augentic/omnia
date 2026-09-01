//! Implementation details for the `#[instrument]` attribute macro.

use quote::quote;
use syn::meta::ParseNestedMeta;
use syn::parse::Result;
use syn::{Expr, ItemFn, LitStr};

pub fn body(attrs: Attributes, item_fn: &ItemFn) -> proc_macro2::TokenStream {
    let name = item_fn.sig.ident.clone();
    let block = item_fn.block.clone();

    let span_name = attrs.name.unwrap_or_else(|| LitStr::new(&name.to_string(), name.span()));
    // All emitted paths route through `omnia_wasi_otel` (the crate the macro
    // is re-exported from), so callers need no direct `tracing` dependency.
    let tracing = quote! { ::omnia_wasi_otel::__private::tracing };
    let level =
        attrs.level.map_or_else(|| quote! { #tracing::Level::INFO }, |level| quote! {#level});

    // `instrument` async functions. The outermost instrumented function owns
    // the telemetry lifecycle (`init` returns its guard exactly once); flush
    // inline before returning so the export completes within this call — a
    // blocking flush in the guard's `Drop` would deadlock the export task.
    if item_fn.sig.asyncness.is_some() {
        quote! {
            let __omnia_otel_guard = ::omnia_wasi_otel::init();
            let __omnia_otel_output = #tracing::Instrument::instrument(
                async move #block,
                #tracing::span!(#level, #span_name)
            ).await;
            ::omnia_wasi_otel::flush_guard(__omnia_otel_guard).await;
            __omnia_otel_output
        }
    } else {
        // A sync function cannot await a flush; the guard's `Drop` defers
        // the export onto the surrounding component-model task.
        quote! {
            let _guard = ::omnia_wasi_otel::init();
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

#[cfg(test)]
mod tests {
    use super::{Attributes, body};

    #[test]
    fn async_fn() {
        let item_fn: syn::ItemFn = syn::parse_quote! {
            async fn handler() { do_work().await }
        };
        let out = body(Attributes::default(), &item_fn).to_string();
        assert!(out.contains("Instrument"), "async body must use Instrument: {out}");
        assert!(out.contains("await"), "async body must be awaited: {out}");
        assert!(
            out.contains("flush_guard (__omnia_otel_guard) . await"),
            "async body must await the telemetry flush before returning: {out}"
        );
    }

    #[test]
    fn sync_fn() {
        let item_fn: syn::ItemFn = syn::parse_quote! {
            fn handler() { do_work() }
        };
        let out = body(Attributes::default(), &item_fn).to_string();
        assert!(out.contains("in_scope"), "sync body must use in_scope: {out}");
        assert!(!out.contains("Instrument"), "sync body must not use Instrument: {out}");
        assert!(!out.contains("flush_guard"), "sync body defers the flush to the guard: {out}");
    }

    #[test]
    fn default_span_name() {
        let item_fn: syn::ItemFn = syn::parse_quote! {
            fn my_handler() {}
        };
        let out = body(Attributes::default(), &item_fn).to_string();
        assert!(out.contains("my_handler"), "span name should default to the fn ident: {out}");
    }
}
