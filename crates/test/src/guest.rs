//! Native doubles for the `omnia_sdk` capability traits.
//!
//! One double per capability, each a plain `Clone + Default` value a
//! handler-level test seeds, hands to the provider under test, and reads
//! back. [`Provider`] bundles all of them behind every capability trait, so
//! a handler generic over its provider runs natively against seeded doubles.
//! [`send`] and [`deliver`] drive one request or one delivery through the
//! router under test.

mod docs;
mod http;
mod memory;
mod provider;
mod router;
mod scripted;
mod sink;
mod tables;
mod vcs;

pub use docs::MemoryDocs;
pub use http::{MatchedHttp, Recorded};
pub use memory::{BlobSnapshot, Memory, Namespaced, StateSnapshot};
pub use provider::Provider;
pub use router::{deliver, send};
pub use scripted::{ScriptedLoader, ScriptedModel, Turn, function_tools};
pub use sink::{Broadcasted, FixedIdentity, MapConfig, Sink};
pub use tables::{Predicate, ScriptedTables, Statement};
pub use vcs::ScriptedVcs;

pub use crate::backend::{SeenVcs, VcsTurn};
