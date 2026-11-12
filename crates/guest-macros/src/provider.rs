//! `#[derive(Provider)]` expansion.

use proc_macro2::{TokenStream, TokenTree};
use quote::{ToTokens, format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{Data, DeriveInput, Error, Field, Fields, Ident, Path, Result, Token, Type, parse_quote};

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

    let assigned = parse_fields(&fields.named)?;

    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let existing_predicates =
        where_clause.map(|wc| wc.predicates.iter().collect::<Vec<_>>()).unwrap_or_default();
    let type_params: Vec<&Ident> = input.generics.type_params().map(|param| &param.ident).collect();

    let impls = assigned.into_iter().map(|(cap, field_ident, field_ty)| {
        let Capability {
            trait_path,
            has_path,
            accessor,
            ..
        } = cap;
        let generic_bound =
            mentions_type_param(field_ty.to_token_stream(), &type_params).then(|| {
                quote! {
                    #field_ty: #trait_path,
                }
            });
        quote! {
            impl #impl_generics #has_path for #struct_name #ty_generics
            where
                #generic_bound
                #(#existing_predicates,)*
            {
                type Inner = #field_ty;

                fn #accessor(&self) -> &#field_ty {
                    &self.#field_ident
                }
            }
        }
    });

    Ok(quote! {
        #(#impls)*
    })
}

pub fn to_snake_case(name: &str) -> String {
    let mut snake = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                snake.push('_');
            }
            snake.push(ch.to_ascii_lowercase());
        } else {
            snake.push(ch);
        }
    }
    snake
}

struct Capability {
    written: Path,
    // the written path without spaces, which a second delegation must not repeat
    text: String,
    trait_path: Path,
    has_path: Path,
    accessor: Ident,
}

impl Capability {
    // A bare name is an `omnia_sdk` capability. A path names a `#[capability]`
    // trait elsewhere, whose `Has<Trait>` sits beside it.
    fn resolve(written: Path) -> Result<Self> {
        let Some(last) = written.segments.last() else {
            return Err(Error::new_spanned(&written, "expected a capability name"));
        };
        if !last.arguments.is_none() {
            return Err(Error::new_spanned(
                &last.arguments,
                "a capability is named without generic arguments",
            ));
        }
        let ident = last.ident.clone();
        let name = ident.to_string();

        let trait_path: Path = if written.leading_colon.is_none() && written.segments.len() == 1 {
            if !CAPABILITIES.contains(&name.as_str()) {
                return Err(Error::new_spanned(
                    &written,
                    format!(
                        "unknown capability `{name}`; expected one of: {}, or the path to a \
                         `#[capability]` trait, such as `crate::{name}`",
                        CAPABILITIES.join(", ")
                    ),
                ));
            }
            parse_quote!(::omnia_sdk::#ident)
        } else {
            written.clone()
        };

        let mut has_path = trait_path.clone();
        if let Some(segment) = has_path.segments.last_mut() {
            segment.ident = format_ident!("Has{}", ident, span = ident.span());
        }
        let accessor = Ident::new(&to_snake_case(&name), ident.span());

        Ok(Self {
            text: path_text(&written),
            written,
            trait_path,
            has_path,
            accessor,
        })
    }
}

fn path_text(path: &Path) -> String {
    path.to_token_stream().to_string().replace(' ', "")
}

fn parse_fields(fields: &Punctuated<Field, Token![,]>) -> Result<Vec<(Capability, &Ident, &Type)>> {
    let mut assigned: Vec<(Capability, &Ident, &Type)> = Vec::new();
    let mut errors: Option<Error> = None;
    let mut push_err = |err: Error| match &mut errors {
        Some(existing) => existing.combine(err),
        None => errors = Some(err),
    };

    for field in fields {
        let Some(field_ident) = &field.ident else {
            continue;
        };
        for attr in &field.attrs {
            if !attr.path().is_ident("provider") {
                continue;
            }
            let list: Punctuated<Path, Token![,]> =
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
            for written in list {
                let cap = match Capability::resolve(written) {
                    Ok(cap) => cap,
                    Err(err) => {
                        push_err(err);
                        continue;
                    }
                };
                let name = &cap.text;
                if let Some((prev_cap, prev_field, _)) =
                    assigned.iter().find(|(prev, _, _)| prev.text == *name)
                {
                    let mut err = Error::new_spanned(
                        &cap.written,
                        format!(
                            "capability `{name}` is already delegated to field `{prev_field}` \
                             (and cannot also be delegated to `{field_ident}`)"
                        ),
                    );
                    err.combine(Error::new_spanned(
                        &prev_cap.written,
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
    Ok(assigned)
}

const CAPABILITIES: &[&str] = &[
    "BlobStore",
    "Config",
    "DocumentDb",
    "HttpClient",
    "Identity",
    "Model",
    "Plugins",
    "Publisher",
    "SecretStore",
    "SqlDb",
    "StateStore",
    "Vcs",
    "WebSockets",
];

fn mentions_type_param(tokens: TokenStream, params: &[&Ident]) -> bool {
    tokens.into_iter().any(|tree| match tree {
        TokenTree::Ident(ident) => params.contains(&&ident),
        TokenTree::Group(group) => mentions_type_param(group.stream(), params),
        TokenTree::Punct(_) | TokenTree::Literal(_) => false,
    })
}
