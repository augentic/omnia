//! The component runtime harness: a manifest-driven [`Deployment`] over a
//! [`Backends`] bundle of the runtime's in-memory defaults, a scripted model
//! backend, and a per-test scratch directory.

mod backends;
mod deployment;
mod model;
mod readers;
mod scratch;

pub use backends::{Backends, STATE_BUCKET};
pub use deployment::Deployment;
pub use model::{Completion, ScriptedModel, Step};
pub use scratch::{Scratch, scratch};
