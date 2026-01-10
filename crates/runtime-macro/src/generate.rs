use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Path};

use crate::runtime::Config;

pub struct Generated {
    pub context_fields: Vec<TokenStream>,
    pub store_ctx_fields: Vec<TokenStream>,
    pub store_ctx_values: Vec<TokenStream>,
    pub host_trait_impls: Vec<Path>,
    pub server_trait_impls: Vec<TokenStream>,
    pub wasi_view_impls: Vec<TokenStream>,
    pub main_fn: TokenStream,
}

impl TryFrom<Config> for Generated {
    type Error = syn::Error;

    fn try_from(input: Config) -> Result<Self, Self::Error> {
        // `Context` struct
        let mut context_fields = Vec::new();
        let mut seen_backends = HashSet::new();

        for backend in input.backends {
            // Deduplicate backends based on their string representation
            let backend_str = quote! {#backend}.to_string();
            if seen_backends.contains(&backend_str) {
                continue;
            }
            seen_backends.insert(backend_str);

            let field = field_ident(&backend);
            context_fields.push(quote! {#field: #backend});
        }

        let mut store_ctx_fields = Vec::new();
        let mut store_ctx_values = Vec::new();
        let mut host_trait_impls = Vec::new();
        let mut server_trait_impls = Vec::new();
        let mut wasi_view_impls = Vec::new();

        for host in &input.hosts {
            let host_type = &host.type_;
            let host_ident = wasi_ident(host_type);
            let backend_type = &host.backend;
            let backend_ident = field_ident(backend_type);

            host_trait_impls.push(host_type.clone());
            store_ctx_fields.push(quote! {#host_ident: #backend_type});
            store_ctx_values.push(quote! {#host_ident: self.#backend_ident.clone()});

            // servers
            server_trait_impls.push(quote! {#host_type});

            // WasiViewXxx implementations
            // HACK: derive module name from WASI type
            let module = wasi_ident(host_type);
            let view = quote! {
                #module::wasi_view!(StoreCtx, #host_ident);
            };
            wasi_view_impls.push(view);
        }

        // main function
        let main_fn = if input.gen_main {
            quote! {
                use yetti::tokio;

                #[tokio::main]
                async fn main() -> anyhow::Result<()> {
                    use yetti::Parser;
                    match yetti::Cli::parse().command {
                        yetti::Command::Run { wasm } => runtime::run(wasm).await,
                        _ => unreachable!(),
                    }
                }
            }
        } else {
            quote! {}
        };

        Ok(Self {
            context_fields,
            store_ctx_fields,
            store_ctx_values,
            host_trait_impls,
            server_trait_impls,
            wasi_view_impls,
            main_fn,
        })
    }
}

/// Generates a field name for a backend type.
fn field_ident(path: &Path) -> Ident {
    let Some(ident) = path.segments.last() else {
        return format_ident!("field");
    };
    let ident_str = quote! {#ident}.to_string();

    // convert the type string to a snake_case
    let mut field_str = String::new();
    for char in ident_str.chars() {
        if char.is_uppercase() {
            if !field_str.is_empty() {
                field_str.push('_');
            }
            field_str.push_str(&char.to_lowercase().to_string());
        } else {
            field_str.push(char);
        }
    }

    format_ident!("{field_str}")
}

fn wasi_ident(path: &Path) -> Ident {
    let Some(ident) = path.segments.last() else {
        return format_ident!("wasi");
    };

    let name = quote! {#ident}.to_string();
    let name = name.replace("Wasi", "yetti_wasi_").to_lowercase();
    format_ident!("{name}")
}
