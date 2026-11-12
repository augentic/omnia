//! `#[derive(FromRow)]` expansion.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields, Result};

pub fn expand(input: &DeriveInput) -> Result<TokenStream> {
    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(
            input,
            "`#[derive(FromRow)]` only supports structs with named fields",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(Error::new_spanned(
            input,
            "`#[derive(FromRow)]` only supports structs with named fields",
        ));
    };

    let reads = fields.named.iter().filter_map(|field| {
        let ident = field.ident.as_ref()?;
        let column = ident.to_string();
        Some(quote! {
            #ident: row.get(#column)?,
        })
    });

    let struct_ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::omnia_sdk::sql::FromRow for #struct_ident #ty_generics #where_clause {
            fn from_row(row: &::omnia_sdk::sql::Row) -> ::omnia_sdk::capability::Result<Self> {
                ::core::result::Result::Ok(Self {
                    #(#reads)*
                })
            }
        }
    })
}
