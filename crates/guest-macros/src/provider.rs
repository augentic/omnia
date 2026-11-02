//! `#[derive(Provider)]` expansion.

use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Data, DeriveInput, Error, Fields, Ident, Result, Token, Type};

const CAPABILITIES: &[&str] = &[
    "BlobStore",
    "Broadcast",
    "Config",
    "DocumentStore",
    "HttpRequest",
    "Identity",
    "Model",
    "Plugins",
    "Publish",
    "StateStore",
    "TableStore",
    "Vcs",
];

pub fn expand(input: &DeriveInput) -> Result<TokenStream> {
    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(
            input,
            "`#[derive(Provider)]` only supports structs with named fields",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(Error::new_spanned(
            input,
            "`#[derive(Provider)]` only supports structs with named fields",
        ));
    };

    let mut assigned: Vec<(Ident, &Ident, &Type)> = Vec::new();
    let mut errors: Option<Error> = None;
    let mut push_err = |err: Error| match &mut errors {
        Some(existing) => existing.combine(err),
        None => errors = Some(err),
    };

    for field in &fields.named {
        let Some(field_ident) = &field.ident else {
            continue;
        };
        for attr in &field.attrs {
            if !attr.path().is_ident("provider") {
                continue;
            }
            let list: Punctuated<Ident, Token![,]> =
                match attr.parse_args_with(Punctuated::parse_terminated) {
                    Ok(list) => list,
                    Err(err) => {
                        push_err(err);
                        continue;
                    }
                };
            if list.is_empty() {
                push_err(Error::new_spanned(
                    attr,
                    "`#[provider(...)]` requires at least one capability name",
                ));
                continue;
            }
            for cap in list {
                let name = cap.to_string();
                if !CAPABILITIES.contains(&name.as_str()) {
                    push_err(Error::new_spanned(
                        &cap,
                        format!(
                            "unknown capability `{name}`; expected one of: {}",
                            CAPABILITIES.join(", ")
                        ),
                    ));
                    continue;
                }
                if let Some((prev_cap, prev_field, _)) =
                    assigned.iter().find(|(prev, _, _)| *prev == name)
                {
                    let mut err = Error::new_spanned(
                        &cap,
                        format!(
                            "capability `{name}` is already delegated to field `{prev_field}` \
                             (and cannot also be delegated to `{field_ident}`)"
                        ),
                    );
                    err.combine(Error::new_spanned(
                        prev_cap,
                        format!("`{name}` first delegated on field `{prev_field}` here"),
                    ));
                    push_err(err);
                    continue;
                }
                assigned.push((cap, field_ident, &field.ty));
            }
        }
    }

    if let Some(err) = errors {
        return Err(err);
    }

    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let existing_predicates =
        where_clause.map(|wc| wc.predicates.iter().collect::<Vec<_>>()).unwrap_or_default();

    let impls = assigned.into_iter().map(|(cap, field_ident, field_ty)| {
        quote! {
            impl #impl_generics ::omnia_sdk::#cap for #struct_name #ty_generics
            where
                Self: ::core::marker::Send + ::core::marker::Sync,
                #field_ty: ::omnia_sdk::#cap,
                #(#existing_predicates,)*
            {
                ::omnia_sdk::__delegate!(@field #cap; self, #field_ident);
            }
        }
    });

    Ok(quote! {
        #(#impls)*
    })
}
