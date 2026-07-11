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

/// The typed outcome supplied to a route projector.
#[derive(Debug)]
pub enum Outcome<T, O, D> {
    /// The operation completed successfully.
    Output(T),
    /// The operation returned its typed failure.
    Operation(O),
    /// The transport input could not be converted to operation input.
    Decode(D),
}

/// Provider trait for application operations.
pub trait Provider: Send + Sync + 'static {}
impl<T> Provider for T where T: Send + Sync + 'static {}
