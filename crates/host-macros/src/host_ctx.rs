//! `#[derive(HostCtx)]` expansion.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Error, Path, Result, Token};

pub fn expand(input: &DeriveInput) -> Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "`#[derive(HostCtx)]` does not support generic types",
        ));
    }

    let host = &input.ident;
    let client = client_path(input)?;

    Ok(quote! {
        impl ::omnia_core::wasmtime::component::HasData for #host {
            type Data<'a> = ::omnia_core::HostView<'a, dyn #client>;
        }

        impl ::omnia_core::HostCtx for #host {
            type Borrow<'a> = &'a dyn #client;

            fn view<'a>(
                borrow: Self::Borrow<'a>,
                table: &'a mut ::omnia_core::wasmtime_wasi::ResourceTable,
            ) -> Self::Data<'a> {
                ::omnia_core::HostView { client: borrow, table }
            }
        }

        impl ::omnia_core::SlotHost for #host {
            fn lend(slot: &mut ::std::sync::Arc<dyn #client>) -> &dyn #client {
                &**slot
            }
        }
    })
}

// The one `#[host_ctx(client = Path)]` attribute; absent, repeated, or any
// other key is an error at the attribute.
fn client_path(input: &DeriveInput) -> Result<Path> {
    let mut client = None;
    for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("host_ctx")) {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("client") {
                return Err(meta.error("unknown `host_ctx` key; expected `client = <Trait>`"));
            }
            if client.is_some() {
                return Err(meta.error("`client` is given more than once"));
            }
            meta.input.parse::<Token![=]>()?;
            client = Some(meta.input.parse::<Path>()?);
            Ok(())
        })?;
    }
    client.ok_or_else(|| {
        Error::new_spanned(
            &input.ident,
            "`#[derive(HostCtx)]` needs `#[host_ctx(client = <Trait>)]` naming the client trait",
        )
    })
}
