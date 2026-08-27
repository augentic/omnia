//! Shared helpers for the guest scenario programs in `programs/`.
//!
//! Compiled only for `wasm32`; the native build of this crate is empty.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Function, Message, Role, Tool};

/// The `verdict` JSON Schema several scenarios request.
pub const VERDICT_SCHEMA: &str =
    r#"{"type":"object","properties":{"verdict":{"type":"string"}},"required":["verdict"]}"#;

/// A `report` JSON Schema whose nested property pins schema error paths.
pub const REPORT_SCHEMA: &str =
    r#"{"type":"object","properties":{"ui-surface":{"type":"object"}}}"#;

/// One user chat turn.
#[must_use]
pub fn user(content: &str) -> Message {
    Message {
        role: Role::User,
        content: content.to_owned(),
    }
}

/// A declared `lookup` function tool.
#[must_use]
pub fn lookup() -> Tool {
    Tool::Function(
        Function::builder().name("lookup").description("test lookup").parameters("{}").build(),
    )
}
