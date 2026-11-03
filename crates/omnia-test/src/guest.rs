//! Native doubles for the `omnia_sdk` capability traits.
//!
//! One double per capability, each a plain `Clone + Default` value a
//! handler-level test seeds, hands to the provider under test, and reads
//! back. [`Provider`] bundles all of them behind every capability trait, so
//! a handler generic over its provider runs natively against seeded doubles.

mod docs;
mod http;
mod memory;
mod provider;
mod scripted;
mod sink;
mod tables;
mod vcs;

pub use docs::MemoryDocs;
pub use http::{MatchedHttp, Recorded};
pub use memory::{BlobSnapshot, Memory, Namespaced, StateSnapshot};
pub use provider::Provider;
#[expect(deprecated, reason = "re-exporting the deprecated alias")]
pub use scripted::Scripted;
pub use scripted::{ScriptedLoader, ScriptedModel, Turn, function_tools};
pub use sink::{Broadcasted, FixedIdentity, MapConfig, Sink};
pub use tables::{Predicate, ScriptedTables, Statement};
pub use vcs::ScriptedVcs;

pub use crate::backend::{SeenVcs, VcsTurn};
