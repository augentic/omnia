mod capabilities;
mod environment;
mod guest;
mod http;
mod messaging;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Generates the guest infrastructure based on the configuration.
///
/// # Example
///
/// ```ignore
/// buildgen::guest!({
///     owner: "at",
///     provider: MyProvider,
///     http: [
///         "/some/path": {
///             method: get,
///             request: SomeRequest,
///             reply: SomeResponse,
///         }
///     ],
///     messaging: [
///         "realtime-r9k.v1": {
///             message: R9kMessage,
///             // optional:
///             // handler: on_realtime_r9k_v1,
///         }
///     ]
/// });
/// ```
///
/// ## Notes
/// - **HTTP**: `handler` should be an Axum-compatible async handler function (the macro wires it into a `Router`).
/// - **Messaging**: if `handler` is omitted, the macro expects a sibling async function named `on_<topic>`,
///   with non-alphanumeric characters replaced by `_` (e.g. `"realtime-r9k.v1"` → `on_realtime_r9k_v1`).
///   The generated code parses payload bytes via `TryFrom<&[u8]>` into `message` and then awaits the handler.
#[proc_macro]
pub fn guest(input: TokenStream) -> TokenStream {
    let config = parse_macro_input!(input as guest::Config);
    guest::expand(&config).into()
}
