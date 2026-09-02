//! Native doubles for the `omnia_guest` capability traits.
//!
//! One double per capability, each a plain `Clone + Default` value a
//! handler-level test seeds, hands to the provider under test, and reads
//! back. `doubles!` assembles a provider from them by capability name;
//! `forward!` delegates a provider's capability impls to its fields.

#[doc(hidden)]
pub mod __forward;
mod docs;
mod http;
mod macros;
mod memory;
mod scripted;
mod sink;
mod tables;

pub use docs::MemoryDocs;
pub use http::{MatchedHttp, Recorded};
pub use memory::{Blobs, Memory, Namespaced, State};
pub use scripted::{Scripted, ScriptedLoader, Turn, function_tools};
pub use sink::{Broadcasted, FixedIdentity, MapConfig, Sink};
pub use tables::{Predicate, ScriptedTables, Statement};

pub use crate::{doubles, forward};
