//! Typed command routing over application operations.

mod builder;
mod response;
mod router;

pub use builder::{Binding, Decoder, Outcome, Projector, Run, TryIntoDecoder, run};
pub use response::CommandResponse;
#[cfg(target_arch = "wasm32")]
pub use router::execute_wasi;
pub use router::{App, BuildError, Completions, Namespace, NoGlobals, RouteInfo, Router, Selector};
