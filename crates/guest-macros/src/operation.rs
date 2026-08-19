//! Implementation details for the `#[operation]` attribute macro.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, FnArg, GenericArgument, Ident, ItemFn, PathArguments, Result, ReturnType, Type};

pub fn expand(item_fn: &ItemFn) -> Result<TokenStream> {
    let sig = &item_fn.sig;
    if sig.asyncness.is_none() {
        return Err(Error::new_spanned(sig.fn_token, "#[operation] requires an `async fn`"));
    }

    let (input_ty, provider) = params(item_fn)?;
    let (output_ty, error_ty) = result_types(&sig.output)?;

    // The impl repeats the fn's generics verbatim. `CallContext<'_, P>` already
    // forces `P: Provider` onto the fn, so the trait's bound is always present.
    let (impl_generics, _, where_clause) = sig.generics.split_for_impl();
    let fn_ident = &sig.ident;

    Ok(quote! {
        #item_fn

        impl #impl_generics ::omnia_guest::api::Operation<#provider> for #input_ty #where_clause {
            type Error = #error_ty;
            type Input = Self;
            type Output = #output_ty;

            async fn call(
                input: Self, context: ::omnia_guest::api::CallContext<'_, #provider>,
            ) -> ::core::result::Result<Self::Output, Self::Error> {
                #fn_ident(input, context).await
            }
        }
    })
}

/// Extract the operation input type and the provider type parameter from the
/// handler's `(input: <InputType>, context: CallContext<'_, P>)` parameters.
fn params(item_fn: &ItemFn) -> Result<(&Type, &Ident)> {
    const SHAPE: &str = "#[operation] expects exactly two parameters: `(input: <InputType>, context: CallContext<'_, P>)`";

    let sig = &item_fn.sig;
    let mut inputs = sig.inputs.iter();
    let ((Some(first), Some(second)), None) = ((inputs.next(), inputs.next()), inputs.next())
    else {
        return Err(Error::new_spanned(&sig.inputs, SHAPE));
    };

    let FnArg::Typed(input) = first else {
        return Err(Error::new_spanned(first, SHAPE));
    };
    let input_ty = input.ty.as_ref();
    if !matches!(input_ty, Type::Path(_)) {
        return Err(Error::new_spanned(
            input_ty,
            "the operation input must be an owned path type; it becomes the `Operation` impl target",
        ));
    }

    let FnArg::Typed(context) = second else {
        return Err(Error::new_spanned(second, SHAPE));
    };
    let provider = context_provider(&context.ty).ok_or_else(|| {
        Error::new_spanned(
            &context.ty,
            "the second parameter must be `CallContext<'_, P>` where `P` is a type parameter of the fn",
        )
    })?;

    Ok((input_ty, provider))
}

/// Return the provider type parameter `P` from a `CallContext<'_, P>` type.
fn context_provider(ty: &Type) -> Option<&Ident> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "CallContext" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let provider = args.args.iter().rev().find_map(|arg| match arg {
        GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })?;
    let Type::Path(provider) = provider else {
        return None;
    };
    provider.path.get_ident()
}

/// Split the handler's `Result` return type into output and error types.
///
/// A single-argument `Result<T>` is read as the `omnia_guest::Result` alias,
/// so the error defaults to `omnia_guest::Error`.
fn result_types(output: &ReturnType) -> Result<(&Type, TokenStream)> {
    const SHAPE: &str =
        "#[operation] handlers must return `Result<T>` (omnia_guest::Result) or `Result<T, E>`";

    let ReturnType::Type(_, ty) = output else {
        return Err(Error::new_spanned(output, SHAPE));
    };
    let Type::Path(path) = ty.as_ref() else {
        return Err(Error::new_spanned(ty, SHAPE));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(Error::new_spanned(ty, SHAPE));
    };
    if segment.ident != "Result" {
        return Err(Error::new_spanned(ty, SHAPE));
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return Err(Error::new_spanned(ty, SHAPE));
    };
    let types: Vec<&Type> = args
        .args
        .iter()
        .filter_map(|arg| match arg {
            GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .collect();
    match types.as_slice() {
        [output] => Ok((output, quote!(::omnia_guest::Error))),
        [output, error] => Ok((output, quote!(#error))),
        _ => Err(Error::new_spanned(ty, SHAPE)),
    }
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::expand;

    #[test]
    fn defaults() {
        let item_fn: syn::ItemFn = parse_quote! {
            pub async fn motion_message<P>(input: MotionMessage, context: CallContext<'_, P>) -> Result<()>
            where
                P: Provider + Config + Publish,
            {
                process(input, context.provider).await
            }
        };
        let out = expand(&item_fn).expect("expands").to_string();

        assert!(
            out.contains(":: omnia_guest :: api :: Operation < P > for MotionMessage"),
            "impl target must be the input type: {out}"
        );
        assert!(out.contains("type Input = Self"), "input must be Self: {out}");
        assert!(out.contains("type Output = ()"), "output from Result<T>: {out}");
        assert!(
            out.contains("type Error = :: omnia_guest :: Error"),
            "error must default to omnia_guest::Error: {out}"
        );
        assert!(
            out.contains("where P : Provider + Config + Publish"),
            "fn bounds must be kept verbatim: {out}"
        );
        assert!(!out.contains("instrument"), "the impl must carry no tracing: {out}");
        assert!(
            out.contains("pub async fn motion_message"),
            "the handler fn must be re-emitted: {out}"
        );
    }

    #[test]
    fn explicit_error_type() {
        let item_fn: syn::ItemFn = parse_quote! {
            async fn lookup<P>(input: Request, context: CallContext<'_, P>) -> Result<Reply, MyError> {
                todo!()
            }
        };
        let out = expand(&item_fn).expect("expands").to_string();

        assert!(out.contains("type Output = Reply"), "output from Result<T, E>: {out}");
        assert!(out.contains("type Error = MyError"), "explicit error must win: {out}");
    }

    #[test]
    fn fn_attributes_re_emitted() {
        let item_fn: syn::ItemFn = parse_quote! {
            #[tracing::instrument(skip_all)]
            async fn handler<P>(input: Message, context: CallContext<'_, P>) -> Result<()> {
                todo!()
            }
        };
        let out = expand(&item_fn).expect("expands").to_string();

        assert!(
            out.contains("# [tracing :: instrument (skip_all , fields (owner = context . owner))]"),
            "user attributes must stay on the re-emitted fn: {out}"
        );
    }

    #[test]
    fn qualified_context_and_result() {
        let item_fn: syn::ItemFn = parse_quote! {
            async fn handler<P>(
                input: Message, context: omnia_guest::api::CallContext<'_, P>,
            ) -> omnia_guest::Result<Reply> {
                todo!()
            }
        };
        let out = expand(&item_fn).expect("expands").to_string();

        assert!(out.contains("type Output = Reply"), "aliased Result<T> output: {out}");
        assert!(
            out.contains("type Error = :: omnia_guest :: Error"),
            "aliased Result<T> error default: {out}"
        );
    }

    #[test]
    fn rejects_sync_fn() {
        let item_fn: syn::ItemFn = parse_quote! {
            fn handler<P>(input: Message, context: CallContext<'_, P>) -> Result<()> {
                todo!()
            }
        };
        let error = expand(&item_fn).expect_err("must reject");

        assert!(error.to_string().contains("async"), "error names the async requirement: {error}");
    }

    #[test]
    fn rejects_missing_context() {
        let item_fn: syn::ItemFn = parse_quote! {
            async fn handler(input: Message) -> Result<()> {
                todo!()
            }
        };
        let error = expand(&item_fn).expect_err("must reject");

        assert!(
            error.to_string().contains("exactly two parameters"),
            "error explains the expected shape: {error}"
        );
    }

    #[test]
    fn rejects_non_result_return() {
        let item_fn: syn::ItemFn = parse_quote! {
            async fn handler<P>(input: Message, context: CallContext<'_, P>) -> u32 {
                todo!()
            }
        };
        let error = expand(&item_fn).expect_err("must reject");

        assert!(
            error.to_string().contains("must return `Result"),
            "error explains the return shape: {error}"
        );
    }

    #[test]
    fn rejects_reference_input() {
        let item_fn: syn::ItemFn = parse_quote! {
            async fn handler<P>(input: &Message, context: CallContext<'_, P>) -> Result<()> {
                todo!()
            }
        };
        let error = expand(&item_fn).expect_err("must reject");

        assert!(
            error.to_string().contains("owned path type"),
            "error explains the input constraint: {error}"
        );
    }
}
