//! `#[derive(HostCtx)]` expansion.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Error, Result};

pub fn expand(input: &DeriveInput) -> Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "`#[derive(HostCtx)]` does not support generic types",
        ));
    }

    let host = &input.ident;
    let name = host.to_string();
    let stem = match name.strip_prefix("Wasi") {
        Some(stem) if !stem.is_empty() => stem,
        _ => {
            return Err(Error::new_spanned(
                host,
                "`#[derive(HostCtx)]` expects a type named `Wasi<Service>`",
            ));
        }
    };

    let vis = &input.vis;
    let view = format_ident!("Wasi{stem}View");
    let client = format_ident!("{stem}Client");
    let view_doc = format!("Borrowed view over a [`{client}`] and the store's resource table.");
    let client_doc = format!("Mutable reference to the WASI {stem} client.");

    Ok(quote! {
        #[doc = #view_doc]
        #vis struct #view<'a> {
            #[doc = #client_doc]
            pub client: &'a mut dyn #client,
            /// Mutable reference to the table used to manage resources.
            pub table: &'a mut ::omnia_core::wasmtime_wasi::ResourceTable,
        }

        impl ::omnia_core::HasTable for #view<'_> {
            fn table(&mut self) -> &mut ::omnia_core::wasmtime_wasi::ResourceTable {
                self.table
            }
        }

        impl ::omnia_core::wasmtime::component::HasData for #host {
            type Data<'a> = #view<'a>;
        }

        impl ::omnia_core::HostCtx for #host {
            type Borrow<'a> = &'a mut dyn #client;

            fn view<'a>(
                borrow: Self::Borrow<'a>,
                table: &'a mut ::omnia_core::wasmtime_wasi::ResourceTable,
            ) -> Self::Data<'a> {
                #view { client: borrow, table }
            }
        }
    })
}
