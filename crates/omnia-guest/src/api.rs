//! Transport-neutral operation invocation and transport adapters.

/// Typed command routing over application operations.
pub mod command;
pub mod http;
/// Typed operation inputs and transport-neutral metadata.
pub mod invocation;
/// Provider-owning invocation primitives.
pub mod invoke;
/// Typed exact-topic messaging routing.
pub mod messaging;
/// Stateless application operations.
pub mod operation;

pub use invocation::{Invocation, Metadata};
pub use invoke::{CallContext, Invoker};
pub use operation::Operation;

/// Provider trait for application operations.
pub trait Provider: Send + Sync + 'static {}
impl<T> Provider for T where T: Send + Sync + 'static {}
