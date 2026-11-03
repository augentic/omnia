//! The component runtime harness: a manifest-driven [`Deployment`] over a
//! [`Backends`] bundle of the runtime's in-memory defaults, a scripted model
//! backend, and a per-test scratch directory.

mod backends;
mod deployment;
mod readers;
mod scratch;

pub use backends::{Backends, NoVcs};
pub use deployment::{Captured, Deployment, Run};
pub use scratch::{Scratch, scratch};

pub use crate::backend::{Completion, ScriptedModel, ScriptedVcs, SeenVcs, Step, VcsTurn};
