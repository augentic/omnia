//! # Model example — session guest
//!
//! A `wasi:cli/command` reactor that **imports** `omnia:model/completion` and
//! drives one completion session when the host calls `wasi:cli/run`.
//!
//! The happy path uses the `omnia-guest` sugar: a tool-less schema completion
//! whose prompt is assembled with `Sections`, lending the `.` mount through
//! `grants.workspace` when the host preopened one.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Format, Message, Model as _, Request, Role, SchemaFormat, WasiModel};
use omnia_wasi_model::prompt::Sections;
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

fn render(outcome: Result<omnia_guest::model::Reply, omnia_guest::model::Error>) -> String {
    match outcome {
        Ok(reply) => reply.answer,
        Err(error) => format!("error: {error:?}"),
    }
}

// A tool-less schema completion over the sugar, lending the `.` mount when
// the host preopened one (with no mount the preopen table is empty and the
// guest lends nothing).
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
        .messages(vec![Message {
            role: Role::User,
            content: user,
        }])
        .format(Format::Schema(
            SchemaFormat::builder().name("verdict").schema("{\"type\":\"object\"}").build(),
        ))
        .maybe_workspace(workspace)
        .build();

    render(WasiModel.complete(request).await)
}
