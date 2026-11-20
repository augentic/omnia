//! # Model example — session guest
//!
//! A `wasi:cli/command` reactor that **imports** `omnia:model` and drives one
//! completion session when the host calls `wasi:cli/run`.
//!
//! The happy path uses the `omnia-sdk` sugar: a tool-less schema completion
//! whose prompt is assembled with `Sections`, lending the `.` mount through
//! `grants.workspace` when the host preopened one.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::prompt::Sections;
use omnia_sdk::model::{Exhausted, Format, Message, Reply, Request, Schema};
use wasip3::exports::cli::run::Guest;
use wasip3::filesystem::preopens;

struct CliGuest;

wasip3::cli::command::export!(CliGuest);

impl Guest for CliGuest {
    async fn run() -> Result<(), ()> {
        println!("{}", default_completion().await);
        Ok(())
    }
}

fn render(outcome: omnia_sdk::Result<Result<Reply, Exhausted>>) -> String {
    match outcome {
        Ok(Ok(reply)) => reply.answer,
        Ok(Err(exhausted)) => format!("no answer: {exhausted}"),
        Err(error) => format!("error: {error:?}"),
    }
}

// a tool-less schema completion, lending the `.` mount when the host preopened one
async fn default_completion() -> String {
    let (system, user) = Sections {
        role: Some("a terse code reviewer".to_string()),
        task: "decide whether the change is acceptable".to_string(),
        context: Some("the diff adds a bounds check".to_string()),
        ..Sections::default()
    }
    .assemble(None);

    let workspace =
        preopens::get_directories().iter().any(|(_, name)| name == ".").then(|| ".".to_owned());

    let request = Request::builder()
        .maybe_system(system)
        .messages(vec![Message::user(user)])
        .format(Format::Schema(Schema {
            name: "verdict".to_owned(),
            schema: r#"{"type":"object"}"#.to_owned(),
        }))
        .maybe_workspace(workspace)
        .build();

    render(request.complete().await)
}
