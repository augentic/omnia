//! Shared helpers for the guest scenario programs in `programs/<capability>/`.
//!
//! Compiled only for `wasm32`; the native build of this crate is empty.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Function, Message, Role, Tool};
use omnia_wasi_model::completion;

/// Wires a scenario `async fn` as the program's `wasi:cli/run` export:
/// `test_programs::run!(scenario);`. The scenario asserts internally;
/// a panic traps the guest and fails the host-side test.
#[macro_export]
macro_rules! run {
    ($scenario:ident) => {
        struct CliGuest;

        ::wasip3::cli::command::export!(CliGuest);

        impl ::wasip3::exports::cli::run::Guest for CliGuest {
            async fn run() -> Result<(), ()> {
                $scenario().await;
                Ok(())
            }
        }
    };
}

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

/// The raw-bindings `lookup` function tool.
#[must_use]
pub fn raw_lookup() -> completion::Function {
    completion::Function {
        name: "lookup".to_owned(),
        description: "test lookup".to_owned(),
        parameters: "{}".to_owned(),
    }
}

/// A minimal raw-bindings request — one `hi` user turn, `format::text` —
/// with the given function tools and grants.
#[must_use]
pub fn raw_request(
    tools: Vec<completion::Function>, grants: completion::Grants,
) -> completion::Request {
    completion::Request {
        model: None,
        system: None,
        messages: vec![completion::Message {
            role: completion::Role::User,
            content: "hi".to_owned(),
        }],
        generation: None,
        format: completion::Format::Text,
        tools: tools.into_iter().map(completion::Tool::Function).collect(),
        grants,
    }
}
