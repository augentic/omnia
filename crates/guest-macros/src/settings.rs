//! `#[derive(Settings)]` expansion.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Error, Field, Fields, GenericArgument, LitStr, PathArguments,
    Result, Type, TypePath,
};

pub fn expand(input: &DeriveInput) -> Result<TokenStream> {
    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(
            input,
            "`#[derive(Settings)]` only supports structs with named fields",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(Error::new_spanned(
            input,
            "`#[derive(Settings)]` only supports structs with named fields",
        ));
    };

    let prefix = struct_prefix(&input.attrs)?;
    let mut keys = Vec::with_capacity(fields.named.len());
    let mut loads = Vec::with_capacity(fields.named.len());

    for field in &fields.named {
        let Some(ident) = &field.ident else {
            continue;
        };
        let key = field_key(field, prefix.as_deref())?;
        let ty = &field.ty;
        keys.push(key.clone());
        let load = optional(ty).map_or_else(
            || quote! { ::omnia_sdk::config::require::<#ty>(#key).await? },
            |inner| quote! { ::omnia_sdk::config::parse::<#inner>(#key).await? },
        );
        loads.push(quote! { #ident: #load, });
    }

    let struct_ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics #struct_ident #ty_generics #where_clause {
            /// The configuration keys [`Self::load`] reads, one per field.
            pub const KEYS: &'static [&'static str] = &[ #(#keys),* ];

            /// Load every field from its key in the deployment's configuration.
            ///
            /// An `Option` field whose key is unset loads as `None`.
            ///
            /// # Errors
            ///
            /// - [`ServerError`](::omnia_sdk::Error::ServerError) when the key
            ///   of a field that is not an `Option` is unset, or a value does not
            ///   parse as its field's type.
            /// - [`BadGateway`](::omnia_sdk::Error::BadGateway) when the
            ///   configuration source does not answer.
            pub async fn load() -> ::omnia_sdk::Result<Self> {
                ::core::result::Result::Ok(Self {
                    #(#loads)*
                })
            }
        }
    })
}

fn struct_prefix(attrs: &[Attribute]) -> Result<Option<String>> {
    let mut prefix = None;
    for attr in attrs {
        if !attr.path().is_ident("setting") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("prefix") {
                let value: LitStr = meta.value()?.parse()?;
                prefix = Some(value.value());
                Ok(())
            } else {
                Err(meta.error("expected `#[setting(prefix = \"...\")]`"))
            }
        })?;
    }
    Ok(prefix)
}

// the key is the field name upper-cased behind the struct's prefix, unless
// `#[setting(key = "..")]` names it outright
fn field_key(field: &Field, prefix: Option<&str>) -> Result<String> {
    let mut explicit = None;
    for attr in &field.attrs {
        if !attr.path().is_ident("setting") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") {
                let value: LitStr = meta.value()?.parse()?;
                explicit = Some(value.value());
                Ok(())
            } else {
                Err(meta.error("expected `#[setting(key = \"...\")]`"))
            }
        })?;
    }

    if let Some(key) = explicit {
        return Ok(key);
    }
    let base = field.ident.as_ref().expect("named field").to_string().to_ascii_uppercase();
    Ok(prefix.map_or_else(|| base.clone(), |prefix| format!("{prefix}{base}")))
}

// the `T` of a field typed `Option<T>`, which reads its key as optional; a
// derive sees only tokens, so an alias of `Option` reads as required
fn optional(ty: &Type) -> Option<&Type> {
    let Type::Path(TypePath {
        qself: None, path, ..
    }) = ty
    else {
        return None;
    };
    let segment = path.segments.last().filter(|segment| segment.ident == "Option")?;
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    match arguments.args.first() {
        Some(GenericArgument::Type(inner)) if arguments.args.len() == 1 => Some(inner),
        _ => None,
    }
}
