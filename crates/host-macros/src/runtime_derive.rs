//! # `Runtime` derive expansion
//!
//! Generates the standard `omnia::Runtime` impl a deployment runtime would
//! otherwise hand-write: `type StoreCtx`, a `registry()` accessor, and a
//! `store()` that builds the fixed `base: omnia::StoreBase` plus one cloned
//! backend per `#[runtime(store = ...)]` field. This is the boilerplate the
//! `runtime!` macro previously emitted inline.
//!
//! ```rust,ignore
//! #[derive(Clone, omnia::Runtime)]
//! #[runtime(store = StoreCtx)]
//! struct Context {
//!     #[runtime(registry)]
//!     registry: Arc<Registry<StoreCtx>>,
//!     #[runtime(store = omnia_wasi_http)]
//!     http_default: HttpDefault,
//! }
//! ```
//!
//! The generated `store()` assumes the `StoreCtx` carries its
//! `omnia::StoreBase` in a field named `base` (the convention the
//! `StoreContext` derive and `runtime!` macro establish).

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned as _;
use syn::{Data, DeriveInput, Fields, Ident, Type};

/// Expand `#[derive(Runtime)]` into an `impl ::omnia::Runtime`.
pub fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let struct_ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let store_ctx = store_ctx_type(input)?;

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(input.span(), "`Runtime` can only be derived for structs"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new(input.span(), "`Runtime` requires a struct with named fields"));
    };

    let mut registry_field: Option<&Ident> = None;
    let mut store_assignments: Vec<TokenStream> = Vec::new();
    let mut seen_targets: Vec<Ident> = Vec::new();

    for field in &fields.named {
        // A named-fields struct always has field idents.
        let Some(field_ident) = field.ident.as_ref() else {
            continue;
        };

        for attr in &field.attrs {
            if !attr.path().is_ident("runtime") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("registry") {
                    if registry_field.is_some() {
                        return Err(meta.error(
                            "duplicate `#[runtime(registry)]`; exactly one field is required",
                        ));
                    }
                    registry_field = Some(field_ident);
                    Ok(())
                } else if meta.path.is_ident("store") {
                    let target: Ident = meta.value()?.parse()?;
                    if seen_targets.contains(&target) {
                        return Err(meta.error(format!(
                            "duplicate `#[runtime(store = {target})]` target field"
                        )));
                    }
                    store_assignments.push(quote! { #target: self.#field_ident.clone() });
                    seen_targets.push(target);
                    Ok(())
                } else {
                    Err(meta.error(
                        "expected `#[runtime(registry)]` or `#[runtime(store = <store field>)]`",
                    ))
                }
            })?;
        }
    }

    let Some(registry_field) = registry_field else {
        return Err(syn::Error::new(
            input.span(),
            "`Runtime` requires exactly one field marked `#[runtime(registry)]`",
        ));
    };

    Ok(quote! {
        impl #impl_generics ::omnia::Runtime for #struct_ident #ty_generics #where_clause {
            type StoreCtx = #store_ctx;

            fn registry(&self) -> &::omnia::Registry<Self::StoreCtx> {
                &self.#registry_field
            }

            fn store(&self) -> Self::StoreCtx {
                #store_ctx {
                    // Fixed per-store state, plus one cloned backend per
                    // `#[runtime(store = ...)]` field.
                    base: ::omnia::StoreBase::new(
                        ::omnia::Runtime::options(self),
                        ::std::sync::Arc::new(::core::clone::Clone::clone(self)),
                    ),
                    #(#store_assignments,)*
                }
            }
        }
    })
}

/// Parse the required struct-level `#[runtime(store = <StoreCtx type>)]`.
fn store_ctx_type(input: &DeriveInput) -> syn::Result<Type> {
    let mut store_ctx: Option<Type> = None;
    for attr in &input.attrs {
        if !attr.path().is_ident("runtime") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("store") {
                if store_ctx.is_some() {
                    return Err(meta.error("duplicate struct-level `#[runtime(store = ...)]`"));
                }
                store_ctx = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error("expected `#[runtime(store = <StoreCtx type>)]`"))
            }
        })?;
    }
    store_ctx.ok_or_else(|| {
        syn::Error::new(
            input.span(),
            "`Runtime` requires a struct-level `#[runtime(store = <StoreCtx type>)]`",
        )
    })
}
