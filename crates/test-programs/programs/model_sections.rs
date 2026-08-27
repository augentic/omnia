//! `Sections` assembly and channels in-guest, round-tripped through the echo.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Model as _, Request, WasiModel};
use omnia_wasi_model::completion::Role as WireRole;
use omnia_wasi_model::prompt::{Example, Sections};
use test_programs::user;

test_programs::command!(scenario);

async fn scenario() {
    let sections = Sections {
        role: Some("a {language} reviewer".to_owned()),
        task: "review the {language} code".to_owned(),
        context: Some("the {language} crate".to_owned()),
        constraints: vec!["be {language}-idiomatic".to_owned()],
        examples: vec![Example {
            input: "in".to_owned(),
            output: "out".to_owned(),
        }],
        variables: vec![("language".to_owned(), "Rust".to_owned())],
    };

    // The preamble is not substituted; it leads the system channel.
    let (system, user_turn) = sections.assemble(Some("prefer {language}"));
    assert_eq!(
        system.as_deref(),
        Some("prefer {language}\n\na Rust reviewer\n\n- be Rust-idiomatic")
    );
    assert_eq!(user_turn, "review the Rust code\n\nthe Rust crate\n\nInput: in\nOutput: out");

    let blank = Sections {
        role: Some("   ".to_owned()),
        task: "do it".to_owned(),
        context: Some(String::new()),
        ..Sections::default()
    };
    let (no_system, task_only) = blank.assemble(None);
    assert!(no_system.is_none());
    assert_eq!(task_only, "do it");

    let (no_system, messages) = blank.channels(None);
    assert!(no_system.is_none());
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, WireRole::User);
    assert_eq!(messages[0].content, "do it");

    let reply = WasiModel
        .complete(Request::builder().maybe_system(system).messages(vec![user(&user_turn)]).build())
        .await
        .expect("echo answers");
    assert_eq!(reply.answer, user_turn);
}
