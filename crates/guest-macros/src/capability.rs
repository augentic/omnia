//! `#[capability]` attribute expansion.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Error, FnArg, Ident, ItemTrait, Pat, Result, TraitItem};

use crate::provider::to_snake_case;

pub fn expand(item_trait: &ItemTrait) -> Result<TokenStream> {
    let vis = &item_trait.vis;
    let trait_ident = &item_trait.ident;
    let trait_name = trait_ident.to_string();
    let has_ident = format_ident!("Has{trait_ident}");
    let accessor_ident = Ident::new(&to_snake_case(&trait_name), trait_ident.span());

    let mut forwarded_methods = Vec::new();
    for item in &item_trait.items {
        let TraitItem::Fn(method) = item else {
            continue;
        };
        let sig = &method.sig;
        let method_ident = &sig.ident;
        let mut arg_idents = Vec::new();
        for input in &sig.inputs {
            match input {
                FnArg::Receiver(_) => {}
                FnArg::Typed(pat_type) => {
                    let Pat::Ident(pat_ident) = &*pat_type.pat else {
                        return Err(Error::new_spanned(
                            &pat_type.pat,
                            "`#[capability]` methods require simple identifier parameters",
                        ));
                    };
                    arg_idents.push(&pat_ident.ident);
                }
            }
        }
        forwarded_methods.push(quote! {
            #sig {
                #has_ident::#accessor_ident(self).#method_ident(#(#arg_idents),*)
            }
        });
    }

    let has_doc = format!("Delegates [`{trait_name}`] to a value this type holds.");
    let accessor_doc = format!("Borrow the inner [`{trait_name}`].");

    Ok(quote! {
        #item_trait

        #[doc = #has_doc]
        #vis trait #has_ident {
            #[doc = #accessor_doc]
            fn #accessor_ident(&self) -> &impl #trait_ident;
        }

        impl<__P: #has_ident + ::core::marker::Send + ::core::marker::Sync> #trait_ident for __P {
            #(#forwarded_methods)*
        }

        impl<__P: #trait_ident> #has_ident for ::std::sync::Arc<__P> {
            fn #accessor_ident(&self) -> &impl #trait_ident {
                &**self
            }
        }

        impl<__P: #trait_ident> #has_ident for &__P {
            fn #accessor_ident(&self) -> &impl #trait_ident {
                &**self
            }
        }

        impl<__P: #trait_ident> #has_ident for ::std::boxed::Box<__P> {
            fn #accessor_ident(&self) -> &impl #trait_ident {
                &**self
            }
        }
    })
}
