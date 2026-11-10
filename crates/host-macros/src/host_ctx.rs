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
    let Args { client, root } = Args::parse(input)?;
    let root = root.map_or_else(|| quote!(::omnia_core), |path| quote!(#path));

    Ok(quote! {
        impl #root::wasmtime::component::HasData for #host {
            type Data<'a> = #root::HostView<'a, dyn #client>;
        }

        impl #root::HostCtx for #host {
            type Borrow<'a> = &'a dyn #client;

            fn view<'a>(
                borrow: Self::Borrow<'a>,
                table: &'a mut #root::wasmtime_wasi::ResourceTable,
            ) -> Self::Data<'a> {
                #root::HostView { client: borrow, table }
            }
        }
    })
}

// The `#[host_ctx(..)]` keys. `root` is the `crate = <path>` the expansion
// names its items under; a proc macro has no `$crate`, so a crate that
// reaches the derive through a re-export has to say where.
struct Args {
    client: Path,
    root: Option<Path>,
}

impl Args {
    fn parse(input: &DeriveInput) -> Result<Self> {
        let mut client = None;
        let mut root = None;
        for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("host_ctx")) {
            attr.parse_nested_meta(|meta| {
                let (key, slot) = if meta.path.is_ident("client") {
                    ("client", &mut client)
                } else if meta.path.is_ident("crate") {
                    ("crate", &mut root)
                } else {
                    return Err(meta.error(
                        "unknown `host_ctx` key; expected `client = <Trait>` or `crate = <path>`",
                    ));
                };
                if slot.is_some() {
                    return Err(meta.error(format!("`{key}` is given more than once")));
                }
                meta.input.parse::<Token![=]>()?;
                *slot = Some(meta.input.parse::<Path>()?);
                Ok(())
            })?;
        }

        let client = client.ok_or_else(|| {
            Error::new_spanned(
                &input.ident,
                "`#[derive(HostCtx)]` needs `#[host_ctx(client = <Trait>)]` naming the client trait",
            )
        })?;
        Ok(Self { client, root })
    }
}
