//! `#[derive(FromEnv)]` expansion.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::meta::ParseNestedMeta;
use syn::punctuated::Punctuated;
use syn::{
    Data, DataStruct, DeriveInput, Error, Field, Fields, GenericArgument, Ident, LitStr, Path,
    PathArguments, Result, Token, Type, TypePath,
};

pub fn expand(input: &DeriveInput) -> Result<TokenStream> {
    let Data::Struct(DataStruct {
        fields: Fields::Named(named),
        ..
    }) = &input.data
    else {
        return Err(Error::new_spanned(
            &input.ident,
            "`#[derive(FromEnv)]` only supports structs with named fields",
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "`#[derive(FromEnv)]` does not support generic types",
        ));
    }

    let Args { root, validate } = Args::parse(input)?;
    let root = root.map_or_else(|| quote!(::omnia_core), |path| quote!(#path));
    let fields = EnvField::parse_all(&named.named)?;
    Ok(Expansion::new(&input.ident, root, validate, fields).tokens())
}

// The struct's `#[env(..)]` keys. `root` is the `crate = <path>` the
// expansion names its items under; a proc macro has no `$crate`, so a crate
// that reaches the derive through a re-export has to say where.
struct Args {
    root: Option<Path>,
    validate: Option<Path>,
}

impl Args {
    fn parse(input: &DeriveInput) -> Result<Self> {
        let mut root = None;
        let mut validate = None;
        for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("env")) {
            attr.parse_nested_meta(|meta| {
                let (key, slot) = if meta.path.is_ident("crate") {
                    ("crate", &mut root)
                } else if meta.path.is_ident("validate") {
                    ("validate", &mut validate)
                } else {
                    return Err(meta.error(
                        "unknown `env` key on a struct; expected `crate = <path>` or \
                         `validate = <path>`",
                    ));
                };
                set(slot, &meta, key, || meta.value()?.parse())
            })?;
        }
        Ok(Self { root, validate })
    }
}

fn set<T>(
    slot: &mut Option<T>, meta: &ParseNestedMeta<'_>, key: &str, value: impl FnOnce() -> Result<T>,
) -> Result<()> {
    if slot.is_some() {
        return Err(meta.error(format!("`{key}` is given more than once")));
    }
    *slot = Some(value()?);
    Ok(())
}

struct EnvField<'a> {
    ident: &'a Ident,
    ty: &'a Type,
    optional: Option<&'a Type>,
    shape: Shape,
    with: Option<Path>,
}

enum Shape {
    Var { name: LitStr, default: Option<LitStr> },
    // `default` alone: a field set in code, which no variable reads
    Code { default: LitStr },
    Nested,
}

impl<'a> EnvField<'a> {
    // every field's error at once, so one build reports them all
    fn parse_all(fields: &'a Punctuated<Field, Token![,]>) -> Result<Vec<Self>> {
        let mut parsed = Vec::with_capacity(fields.len());
        let mut errors: Option<Error> = None;
        for field in fields {
            match Self::parse(field) {
                Ok(field) => parsed.push(field),
                Err(err) => match &mut errors {
                    Some(errors) => errors.combine(err),
                    None => errors = Some(err),
                },
            }
        }
        errors.map_or(Ok(parsed), Err)
    }

    fn parse(field: &'a Field) -> Result<Self> {
        let ident = field.ident.as_ref().expect("named field");
        let mut from: Option<LitStr> = None;
        let mut default: Option<LitStr> = None;
        let mut with: Option<Path> = None;
        let mut nested: Option<Path> = None;
        for attr in field.attrs.iter().filter(|attr| attr.path().is_ident("env")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("from") {
                    if !meta.input.peek(Token![=]) {
                        return Err(
                            meta.error("`from` names its variable, as in `from = \"NAME\"`")
                        );
                    }
                    set(&mut from, &meta, "from", || meta.value()?.parse())
                } else if meta.path.is_ident("default") {
                    set(&mut default, &meta, "default", || meta.value()?.parse())
                } else if meta.path.is_ident("with") {
                    set(&mut with, &meta, "with", || meta.value()?.parse())
                } else if meta.path.is_ident("nested") {
                    if meta.input.peek(Token![=]) {
                        return Err(meta.error("`nested` takes no value"));
                    }
                    set(&mut nested, &meta, "nested", || Ok(meta.path.clone()))
                } else {
                    Err(meta.error(
                        "unknown `env` key; expected `from`, `default`, `with`, or `nested`",
                    ))
                }
            })?;
        }

        let shape = match (nested, from, default) {
            (Some(nested), from, default) => {
                if from.is_some() || default.is_some() || with.is_some() {
                    return Err(Error::new_spanned(nested, "`nested` takes no other key"));
                }
                Shape::Nested
            }
            (None, Some(name), default) => {
                if name.value().is_empty() {
                    return Err(Error::new_spanned(name, "`from` names a variable"));
                }
                Shape::Var { name, default }
            }
            (None, None, Some(default)) => Shape::Code { default },
            (None, None, None) => {
                return Err(Error::new_spanned(
                    ident,
                    "a `FromEnv` field takes `#[env(from = \"NAME\")]` to read a variable, \
                     `#[env(default = \"..\")]` to be set in code alone, or `#[env(nested)]`",
                ));
            }
        };

        let optional = optional(&field.ty);
        if optional.is_some()
            && let Shape::Var {
                default: Some(default),
                ..
            }
            | Shape::Code { default } = &shape
        {
            return Err(Error::new_spanned(
                default,
                "an `Option` field takes no `default`, since `None` is its default",
            ));
        }
        Ok(Self {
            ident,
            ty: &field.ty,
            optional,
            shape,
            with,
        })
    }

    // the fn that parses a value of the field, or of its `Option`'s `T`
    fn parser(&self) -> TokenStream {
        if let Some(with) = &self.with {
            return quote!(#with);
        }
        let ty = self.optional.unwrap_or(self.ty);
        quote!(<#ty as ::core::str::FromStr>::from_str)
    }

    const fn default(&self) -> Option<&LitStr> {
        match &self.shape {
            Shape::Var { default, .. } => default.as_ref(),
            Shape::Code { default } => Some(default),
            Shape::Nested => None,
        }
    }
}

// the `T` of a field typed `Option<T>`; a derive sees only tokens, so an
// alias of `Option` reads as required
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

// The names one expansion threads through every item it emits. The locals are
// `mixed_site`, so a field named `source` or a `with` fn named like a field
// resolves as its author wrote it.
struct Expansion<'a> {
    ident: &'a Ident,
    root: TokenStream,
    support: TokenStream,
    validate: Option<Path>,
    fields: Vec<EnvField<'a>>,
    base: Ident,
    source: Ident,
    report: Ident,
}

impl<'a> Expansion<'a> {
    fn new(
        ident: &'a Ident, root: TokenStream, validate: Option<Path>, fields: Vec<EnvField<'a>>,
    ) -> Self {
        Self {
            ident,
            support: quote!(#root::env::derive),
            root,
            validate,
            fields,
            base: Ident::new("base", Span::mixed_site()),
            source: Ident::new("source", Span::mixed_site()),
            report: Ident::new("report", Span::mixed_site()),
        }
    }

    fn tokens(&self) -> TokenStream {
        let Self {
            ident, root, support, ..
        } = self;
        let loaders = self.loaders();
        let fields = self.fields_impl();
        let default = self.default_impl();
        quote! {
            #loaders

            #[automatically_derived]
            impl #root::FromEnv for #ident {
                fn load_env() -> #root::anyhow::Result<Self> {
                    #support::load(::core::option::Option::None, &#support::Source::Process)
                }
            }

            #fields
            #default
        }
    }

    fn loaders(&self) -> TokenStream {
        let Self {
            ident, root, support, ..
        } = self;
        let mut errors = vec![
            " # Errors".to_owned(),
            String::new(),
            " Fails with one error naming every required variable that is unset and".to_owned(),
            " every variable whose value does not parse, but never a value.".to_owned(),
        ];
        if self.validate.is_some() {
            errors.push(
                " Fails with the error of `validate` when it refuses the options.".to_owned(),
            );
        }

        quote! {
            impl #ident {
                /// Loads these options from the process environment.
                ///
                /// A field whose variable is unset takes its `default`, or `None`
                /// when it is an `Option`.
                ///
                #(#[doc = #errors])*
                pub fn load_env() -> #root::anyhow::Result<Self> {
                    #support::load(::core::option::Option::None, &#support::Source::Process)
                }

                /// Applies each variable that is set over these options.
                ///
                /// A field whose variable is unset keeps its value, so a value set
                /// in code yields only to a variable an operator sets.
                ///
                #(#[doc = #errors])*
                pub fn with_env(self) -> #root::anyhow::Result<Self> {
                    #support::load(::core::option::Option::Some(self), &#support::Source::Process)
                }

                /// Loads these options through `lookup` in place of the process environment.
                ///
                /// `lookup` answers a variable's value, or `None` when it is unset.
                ///
                #(#[doc = #errors])*
                pub fn load_from(
                    lookup: impl ::core::ops::Fn(&str) -> ::core::option::Option<::std::string::String>,
                ) -> #root::anyhow::Result<Self> {
                    #support::load(
                        ::core::option::Option::None,
                        &#support::Source::Lookup(&lookup),
                    )
                }
            }
        }
    }

    fn fields_impl(&self) -> TokenStream {
        let Self {
            ident,
            support,
            base,
            source,
            report,
            ..
        } = self;
        let value = Ident::new("value", Span::mixed_site());
        let names: Vec<_> = self.fields.iter().map(|field| field.ident).collect();
        let bindings: Vec<_> = self
            .fields
            .iter()
            .map(|field| format_ident!("field_{}", field.ident, span = Span::mixed_site()))
            .collect();
        let nones = bindings.iter().map(|_| quote!(::core::option::Option::None));
        let loads =
            self.fields.iter().zip(&bindings).map(|(field, binding)| self.load(field, binding));
        let build = self.validate.as_ref().map_or_else(
            || quote!(::core::option::Option::Some(#value)),
            |validate| quote!(#support::check(#value, #report, #validate)),
        );

        quote! {
            #[automatically_derived]
            impl #support::Fields for #ident {
                fn load(
                    #base: ::core::option::Option<Self>,
                    #source: &#support::Source<'_>,
                    #report: &mut #support::Report,
                ) -> ::core::option::Option<Self> {
                    let (#(#bindings,)*) = match #base {
                        ::core::option::Option::Some(Self { #(#names: #bindings),* }) => {
                            (#(::core::option::Option::Some(#bindings),)*)
                        }
                        ::core::option::Option::None => (#(#nones,)*),
                    };
                    #(let #bindings = #loads;)*
                    let #value = Self { #(#names: #bindings?),* };
                    #build
                }
            }
        }
    }

    // one field over its `binding`, which holds the field's value in a load
    // over a value and `None` in any other
    fn load(&self, field: &EnvField<'_>, binding: &Ident) -> TokenStream {
        let Self {
            support,
            source,
            report,
            ..
        } = self;
        let parse = field.parser();
        let label = self.label(field);
        match (&field.shape, field.optional) {
            (Shape::Var { name, default: None }, Some(_)) => {
                quote!(#support::optional(#source, #report, #name, #parse, #binding))
            }
            (Shape::Var { name, default: None }, None) => {
                quote!(#support::required(#source, #report, #name, #parse, #binding))
            }
            (
                Shape::Var {
                    name,
                    default: Some(default),
                },
                _,
            ) => quote! {
                #support::defaulted(#source, #report, #name, #parse, #binding, #label, #default)
            },
            (Shape::Code { default }, _) => {
                quote!(#support::code(#binding, #label, #default, #parse))
            }
            (Shape::Nested, Some(inner)) => {
                quote!(#support::nested::<#inner>(#binding, #source, #report))
            }
            (Shape::Nested, None) => {
                let ty = field.ty;
                quote!(<#ty as #support::Fields>::load(#binding, #source, #report))
            }
        }
    }

    // `Default` exists only where every field has a `default` or is an `Option`
    fn default_impl(&self) -> Option<TokenStream> {
        let Self { ident, support, .. } = self;
        let values = self
            .fields
            .iter()
            .map(|field| {
                let name = field.ident;
                if field.optional.is_some() {
                    return Some(quote!(#name: ::core::option::Option::None));
                }
                let default = field.default()?;
                let label = self.label(field);
                let parse = field.parser();
                Some(quote!(#name: #support::default(#label, #default, #parse)))
            })
            .collect::<Option<Vec<_>>>()?;

        Some(quote! {
            #[automatically_derived]
            impl ::core::default::Default for #ident {
                fn default() -> Self {
                    Self { #(#values),* }
                }
            }
        })
    }

    fn label(&self, field: &EnvField<'_>) -> String {
        format!("{}.{}", self.ident, field.ident)
    }
}
