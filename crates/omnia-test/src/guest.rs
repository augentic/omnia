//! Native doubles for the `omnia_guest` capability traits.
//!
//! One double per capability, each a plain `Clone + Default` value a
//! handler-level test seeds, hands to the provider under test, and reads
//! back. `provider!` — the native twin of `omnia_guest::provider!` —
//! assembles a provider from them by capability name; `delegate!` delegates
//! a hand-written provider's capability impls to its fields.

#[doc(hidden)]
pub mod __delegate;
mod docs;
mod http;
mod macros;
mod memory;
mod scripted;
mod sink;
mod tables;

pub use docs::MemoryDocs;
pub use http::{MatchedHttp, Recorded};
pub use memory::{BlobSnapshot, Memory, Namespaced, StateSnapshot};
pub use scripted::{Scripted, ScriptedLoader, Turn, function_tools};
pub use sink::{Broadcasted, FixedIdentity, MapConfig, Sink};
pub use tables::{Predicate, ScriptedTables, Statement};

pub use crate::{delegate, provider};
