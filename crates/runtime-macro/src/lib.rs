//! Procedural macros for the qwasr runtime.

mod expand;
mod runtime;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Generates the runtime infrastructure based on the configuration.
///
/// # Example
///
/// ```ignore
/// qwasr::runtime!({
///     qwasr_wasi_http: WasiHttp,
///     qwasr_wasi_otel: DefaultOtel,
///     qwasr_wasi_blobstore: MongoDb,
/// });
/// ```
#[proc_macro]
pub fn runtime(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as runtime::Config);
    match expand::expand(&parsed) {
        Ok(ts) => ts.into(),
        Err(e) => e.into_compile_error().into(),
    }
}
