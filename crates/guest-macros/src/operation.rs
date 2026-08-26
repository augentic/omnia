//! Implementation details for the `#[operation]` attribute macro.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Error, FnArg, GenericArgument, Ident, ItemFn, PathArguments, Result, ReturnType, Signature,
    Type,
};

pub fn expand(item_fn: &ItemFn) -> Result<TokenStream> {
    let sig = &item_fn.sig;
    if sig.asyncness.is_none() {
        return Err(Error::new_spanned(sig.fn_token, "#[operation] requires an `async fn`"));
    }

    let (input_ty, provider) = params(sig)?;
    let (output_ty, error_ty) = result_types(&sig.output)?;

    let (impl_generics, _, where_clause) = sig.generics.split_for_impl();
    let fn_ident = &sig.ident;

    Ok(quote! {
        #item_fn

        impl #impl_generics ::omnia_guest::api::Handler<#provider> for #input_ty #where_clause {
            type Error = #error_ty;
            type Output = #output_ty;

            async fn handle(
                self, context: ::omnia_guest::api::Context<'_, #provider>,
            ) -> ::core::result::Result<Self::Output, Self::Error> {
                #fn_ident(self, context).await
            }
        }
    })
}

fn params(sig: &Signature) -> Result<(&Type, &Ident)> {
    const SHAPE: &str = "#[operation] expects exactly two parameters: `(input: <InputType>, context: Context<'_, P>)`";

    let mut inputs = sig.inputs.iter();
    let (Some(first), Some(second), None) = (inputs.next(), inputs.next(), inputs.next()) else {
        return Err(Error::new_spanned(&sig.inputs, SHAPE));
    };

    let FnArg::Typed(input) = first else {
        return Err(Error::new_spanned(first, SHAPE));
    };
    let input_ty = input.ty.as_ref();
    if !matches!(input_ty, Type::Path(_)) {
        return Err(Error::new_spanned(
            input_ty,
            "the operation input must be an owned path type; it becomes the `Handler` impl target",
        ));
    }

    let FnArg::Typed(context) = second else {
        return Err(Error::new_spanned(second, SHAPE));
    };
    let provider = call_context(&context.ty).ok_or_else(|| {
        Error::new_spanned(
            &context.ty,
            "the second parameter must be `Context<'_, P>` where `P` is a type parameter of the fn",
        )
    })?;

    Ok((input_ty, provider))
}

fn call_context(ty: &Type) -> Option<&Ident> {
    let (name, args) = path_types(ty)?;
    if *name != "Context" {
        return None;
    }
    match args.last()? {
        Type::Path(provider) => provider.path.get_ident(),
        _ => None,
    }
}

fn result_types(output: &ReturnType) -> Result<(&Type, TokenStream)> {
    const SHAPE: &str =
        "#[operation] handlers must return `Result<T>` (omnia_guest::Result) or `Result<T, E>`";

    let ReturnType::Type(_, ty) = output else {
        return Err(Error::new_spanned(output, SHAPE));
    };
    let Some((name, args)) = path_types(ty) else {
        return Err(Error::new_spanned(ty, SHAPE));
    };
    if *name != "Result" {
        return Err(Error::new_spanned(ty, SHAPE));
    }
    match args.as_slice() {
        // One-arg Result<T> is the omnia_guest::Result alias.
        [output] => Ok((output, quote!(::omnia_guest::Error))),
        [output, error] => Ok((output, quote!(#error))),
        _ => Err(Error::new_spanned(ty, SHAPE)),
    }
}

fn path_types(ty: &Type) -> Option<(&Ident, Vec<&Type>)> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let types = args
        .args
        .iter()
        .filter_map(|arg| match arg {
            GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .collect();
    Some((&segment.ident, types))
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::expand;

    #[test]
    fn defaults() {
        let item_fn: syn::ItemFn = parse_quote! {
            async fn motion_message<P>(input: MotionMessage, context: Context<'_, P>) -> Result<()> {
                todo!()
            }
        };
        let out = expand(&item_fn).expect("expands").to_string();

        assert!(
            out.contains(":: omnia_guest :: api :: Handler < P > for MotionMessage"),
            "impl target must be the input type: {out}"
        );
        assert!(out.contains("type Output = ()"), "output from Result<T>: {out}");
        assert!(
            out.contains("type Error = :: omnia_guest :: Error"),
            "error must default to omnia_guest::Error: {out}"
        );
    }

    #[test]
    fn explicit_error_type() {
        let item_fn: syn::ItemFn = parse_quote! {
            async fn lookup<P>(input: Request, context: Context<'_, P>) -> Result<Reply, MyError> {
                todo!()
            }
        };
        let out = expand(&item_fn).expect("expands").to_string();

        assert!(out.contains("type Output = Reply"), "output from Result<T, E>: {out}");
        assert!(out.contains("type Error = MyError"), "explicit error must win: {out}");
    }
}
