//! Wasmtime-free `*Client` test doubles (`ScriptedModel` and `ScriptedVcs`).

mod model;
mod vcs;

pub use model::{Completion, ScriptedModel, Step};
pub use vcs::{ScriptedVcs, SeenVcs, VcsTurn};
