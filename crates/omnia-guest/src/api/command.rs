//! Typed command routing over application operations.

mod builder;
mod response;
#[cfg(feature = "cli")]
mod router;

pub use builder::{Binding, Decoder, Outcome, Projector, Run, TryIntoDecoder, run};
pub use response::CommandResponse;
#[cfg(all(target_arch = "wasm32", feature = "cli"))]
pub use router::execute_wasi;
#[cfg(feature = "cli")]
pub use router::{
    BuildError, Completions, Namespace, NoGlobals, RouteInfo, Router, RouterBuilder, Selector,
};
