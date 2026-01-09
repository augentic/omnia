use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitStr, Result, Token, Type};

pub struct Environment {
    pub vars: Vec<EnvVar>,
}

impl Parse for Environment {
    fn parse(input: ParseStream) -> Result<Self> {
        let vars = Punctuated::<EnvVar, Token![,]>::parse_terminated(input)?;
        Ok(Self {
            vars: vars.into_iter().collect(),
        })
    }
}

pub struct EnvVar {
    pub name: Ident,
    pub ty: Type,
    pub default: Option<LitStr>,
}

impl Parse for EnvVar {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty: Type = input.parse()?;

        let default = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Some(input.parse::<LitStr>()?)
        } else {
            None
        };

        Ok(Self { name, ty, default })
    }
}

pub fn expand(environment: &Environment) -> TokenStream {
    let fields = environment.vars.iter().map(expand_field);
    let match_arms = environment.vars.iter().map(expand_match_arm);

    quote! {
        mod environment {
            use warp_sdk::anyhow::Result;
            use warp_sdk::fromenv::FromEnv;
            use warp_sdk::Config;

            use super::*;

            #[derive(Debug, Clone, FromEnv)]
            pub struct ConfigSettings {
                #(#fields)*
            }

            impl Default for ConfigSettings {
                fn default() -> Self {
                    // we panic here to ensure configuration is always loaded
                    // i.e. guest should not start without proper configuration
                    Self::from_env().finalize().expect("should load configuration")
                }
            }

            impl Config for ConfigSettings {
                async fn get(&self, key: &str) -> Result<String> {
                    Ok(match key {
                        #(#match_arms)*
                        _ => return Err(warp_sdk::anyhow::anyhow!("unknown config key: {key}")),
                    }
                    .clone())
                }
            }
        }
    }
}

fn expand_field(var: &EnvVar) -> TokenStream {
    let name = &var.name;
    let field_name = to_snake_case(&name.to_string());
    let field_ident = format_ident!("{}", field_name);
    let ty = &var.ty;
    let env_name = name.to_string();

    let attr = var.default.as_ref().map_or_else(
        || quote! { #[env(from = #env_name)] },
        |default| quote! { #[env(from = #env_name, default = #default)] },
    );

    quote! {
        #attr
        pub #field_ident: #ty,
    }
}

fn expand_match_arm(var: &EnvVar) -> TokenStream {
    let name = &var.name;
    let field_name = to_snake_case(&name.to_string());
    let field_ident = format_ident!("{}", field_name);
    let env_name = name.to_string();

    quote! {
        #env_name => &self.#field_ident,
    }
}

/// Convert an `UPPER_SNAKE_CASE` identifier to `lower_snake_case`.
fn to_snake_case(s: &str) -> String {
    s.to_lowercase()
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::*;

    #[test]
    fn parse_environment() {
        let input = quote! {
            ENV: String = "dev",
            BLOCK_MGT_URL: String,
            CC_STATIC_URL: String,
        };

        let parsed: Environment = syn::parse2(input).expect("should parse");
        assert_eq!(parsed.vars.len(), 3);

        assert_eq!(parsed.vars[0].name.to_string(), "ENV");
        assert!(parsed.vars[0].default.is_some());
        assert_eq!(parsed.vars[0].default.as_ref().unwrap().value(), "dev");

        assert_eq!(parsed.vars[1].name.to_string(), "BLOCK_MGT_URL");
        assert!(parsed.vars[1].default.is_none());
    }

    #[test]
    fn to_snake_case_conversion() {
        assert_eq!(to_snake_case("ENV"), "env");
        assert_eq!(to_snake_case("BLOCK_MGT_URL"), "block_mgt_url");
        assert_eq!(to_snake_case("CC_STATIC_URL"), "cc_static_url");
    }
}
