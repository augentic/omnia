//! The component runtime harness and the backends a deployment runs over.
//!
//! A manifest-driven [`Deployment`] boots over a [`Backends`] bundle of the
//! runtime's in-memory defaults, each swappable for a scripted or recording
//! backend. [`send`] and [`deliver`] drive one trigger in process, and
//! [`scratch`] gives each test a directory of its own.

mod backends;
mod deployment;
mod http;
mod model;
mod precompiled;
mod readers;
mod scratch;
mod sink;
mod tables;
mod trigger;

pub use backends::{Backends, NoVcs, STATE_BUCKET};
pub use deployment::{Captured, Deployment, Run};
pub use http::{MatchedHttp, Recorded};
pub use model::{Completion, ScriptedModel, Step};
pub use omnia_wasi_messaging::HandleError;
pub use precompiled::precompiled;
pub use scratch::{Scratch, scratch};
pub use sink::{Asked, Broadcasted, FixedIdentity, Sink};
pub use tables::{Ended, Executed, Predicate, ScriptedTables, Transacted};
pub use trigger::{deliver, send};

/// The host-side `omnia:sql` values a [`ScriptedTables`] rule matches and
/// answers with.
pub mod sql {
    pub use omnia_wasi_sql::{Error, Field, Row, Value};
}
