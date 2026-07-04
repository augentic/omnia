//! # WASI Model Guest
//!
//! Guest-side bindings for the `omnia:model` world. A guest imports
//! `omnia:model/completion` and calls `create`. A structured prompt template
//! is assembled into the request's `system` / `messages` channels with
//! [`crate::prompt::Sections`] before the call.

mod model {
    #![allow(missing_docs)]
    wit_bindgen::generate!({
        world: "model",
        path: "wit",
        with: {
            "wasi:filesystem/types@0.3.0": wasip3::filesystem::types,
            "wasi:clocks/system-clock@0.3.0": wasip3::clocks::system_clock,
            "wasi:clocks/types@0.3.0": wasip3::clocks::types,
        },
    });
}

use self::model::omnia::model::completion::{Message, Role};
pub use self::model::omnia::model::*;

impl crate::prompt::Sections {
    /// Assemble the template into the request's chat channels: the system
    /// string (led by `preamble` when given) and a single user turn.
    #[must_use]
    pub fn channels(&self, preamble: Option<&str>) -> (Option<String>, Vec<Message>) {
        let (system, user) = self.assemble(preamble);
        (
            system,
            vec![Message {
                role: Role::User,
                content: user,
            }],
        )
    }
}
