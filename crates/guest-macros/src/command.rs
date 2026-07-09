use proc_macro2::TokenStream;
use quote::quote;
use syn::Path;
use syn::parse::{Parse, ParseStream, Result};

/// The `command:` trigger: an app-supplied async dispatch function
/// (`async fn(Vec<String>) -> u8`) wired to the `wasi:cli/run` export.
pub struct Command {
    pub dispatch: Path,
}

impl Parse for Command {
    fn parse(input: ParseStream) -> Result<Self> {
        let dispatch: Path = input.parse()?;
        Ok(Self { dispatch })
    }
}

pub fn expand(command: &Command) -> TokenStream {
    let dispatch = &command.dispatch;

    quote! {
        #[cfg(target_arch = "wasm32")]
        mod command {
            use omnia_guest::wasip3;

            use super::*;

            pub struct Command;
            wasip3::cli::command::export!(Command);

            impl wasip3::exports::cli::run::Guest for Command {
                async fn run() -> Result<(), ()> {
                    // argv verbatim as the host provides it, argv[0]
                    // included — the app-side dispatch sees exactly what a
                    // native process would.
                    let argv = wasip3::cli::environment::get_arguments();
                    let code: u8 = #dispatch(argv).await;
                    if code == 0 {
                        return Ok(());
                    }
                    // Exit-code passthrough: exit-with-code does not
                    // return; the Err leg only pacifies the type.
                    wasip3::cli::exit::exit_with_code(code);
                    Err(())
                }
            }
        }
    }
}
