//! What the backend answers reaches the guest over a chain of completions.
//!
//! - Without a check, a `format::schema` answer passes through as the
//!   backend produced it: the schema steers the provider, nothing gates.
//! - A backend-reported usage record is projected onto the guest reply.
//! - `Sections` assembly and channels in-guest, round-tripped through the
//!   backend.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::prompt::{Example, Sections};
use omnia_sdk::model::{Message, Request, Role};
use test_programs::{VERDICT_SCHEMA, user, verdict_format};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    schema_answer().await;
    usage().await;
    sections().await;
}

async fn schema_answer() {
    let request = Request::builder()
        .messages(vec![user("hi")])
        .format(verdict_format(VERDICT_SCHEMA))
        .build();

    let reply =
        request.complete().await.expect("the completion runs").expect("the answer passes through");
    assert_eq!(reply.answer, r#"{"verdict":"pass"}"#);
}

async fn usage() {
    let reply = Request::builder()
        .messages(vec![user("hi")])
        .build()
        .complete()
        .await
        .expect("the completion runs")
        .expect("canned answer carries usage");
    assert_eq!(reply.answer, "hi");

    let usage = reply.usage.expect("the backend reports usage");
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.reasoning_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens,
        ),
        (3, 5, Some(1), Some(2), None)
    );
}

async fn sections() {
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
    assert!(
        matches!(messages.as_slice(), [Message { role: Role::User, content }] if content == "do it"),
        "unexpected: {messages:?}"
    );

    let (system, messages) = sections.channels(Some("prefer {language}"));
    let reply = Request::builder()
        .maybe_system(system)
        .messages(messages)
        .build()
        .complete()
        .await
        .expect("the completion runs")
        .expect("the backend answers");
    assert_eq!(reply.answer, user_turn);
}
