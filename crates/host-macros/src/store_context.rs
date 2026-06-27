//! # `StoreContext` derive expansion
//!
//! Generates the three fixed store-context trait impls (`WasiView`,
//! `WrpcView`, `HasLimits`) against the `#[base]` field of type `omnia::StoreBase`,
//! plus one host view (`<crate>::omnia_wasi_view!`) per `#[wasi(path)]` backend
//! field. This is the per-store boilerplate the `runtime!` macro and
//! hand-written runtimes previously reproduced by hand.

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned as _;
use syn::{Data, DeriveInput, Fields, Ident, Path};

/// Expand `#[derive(StoreContext)]` into the fixed trait impls + host views.
pub fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let struct_ident = &input.ident;

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.span(),
            "`StoreContext` can only be derived for structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new(
            input.span(),
            "`StoreContext` requires a struct with named fields",
        ));
    };

    let mut base_field: Option<&Ident> = None;
    let mut wasi_fields: Vec<(&Ident, Path)> = Vec::new();

    for field in &fields.named {
        // A named-fields struct always has field idents.
        let Some(ident) = field.ident.as_ref() else {
            continue;
        };

        for attr in &field.attrs {
            if attr.path().is_ident("base") {
                if base_field.is_some() {
                    return Err(syn::Error::new(
                        attr.span(),
                        "duplicate `#[base]` field; exactly one is required",
                    ));
                }
                base_field = Some(ident);
            } else if attr.path().is_ident("wasi") {
                let path: Path = attr.parse_args().map_err(|err| {
                    syn::Error::new(
                        attr.span(),
                        format!(
                            "`#[wasi(...)]` expects a host-crate path, e.g. \
                             `#[wasi(omnia_wasi_http)]`: {err}"
                        ),
                    )
                })?;
                wasi_fields.push((ident, path));
            }
        }
    }

    let Some(base) = base_field else {
        return Err(syn::Error::new(
            input.span(),
            "`StoreContext` requires exactly one field marked `#[base]`",
        ));
    };

    // One host view per backend field, delegating to the host crate's
    // `omnia_wasi_view!` macro (which now reaches for `self.base.*`).
    let host_views = wasi_fields.iter().map(|(field, path)| {
        quote! {
            #path::omnia_wasi_view!(#struct_ident, #field);
        }
    });

    Ok(quote! {
        impl ::omnia::wasmtime_wasi::WasiView for #struct_ident {
            fn ctx(&mut self) -> ::omnia::wasmtime_wasi::WasiCtxView<'_> {
                ::omnia::wasmtime_wasi::WasiCtxView {
                    ctx: &mut self.#base.wasi,
                    table: &mut self.#base.table,
                }
            }
        }

        impl ::omnia::WrpcView for #struct_ident {
            type Invoke = ::omnia::LinkClient;

            fn wrpc(&mut self) -> ::omnia::WrpcCtxView<'_, ::omnia::LinkClient> {
                self.#base.wrpc.view(&mut self.#base.table)
            }
        }

        impl ::omnia::HasLimits for #struct_ident {
            fn limits(&mut self) -> &mut ::omnia::wasmtime::StoreLimits {
                &mut self.#base.limits
            }
        }

        #(#host_views)*
    })
}
