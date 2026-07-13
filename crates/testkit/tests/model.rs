//! Public model test-double behavior.

use futures::executor::block_on;
use omnia_guest::model::{
    Effort, Error, Format, Function, Generation, McpGrant, Message, Model, Reply, Request, Role,
    SchemaFormat, Tool, Usage,
};
use omnia_testkit::model::{Harness, Scripted, mcp_grants};

#[test]
fn scripting_order() {
    let model = Scripted::answers(["first", "second"]);

    assert_eq!(complete(&model, request("one")).unwrap().answer, "first");
    assert_eq!(complete(&model, request("two")).unwrap().answer, "second");
    model.assert_exhausted();
}

#[test]
fn error_injection() {
    let expected = Error::ToolFailed("resolver unavailable".to_owned());
    let model = Scripted::new([
        Err(expected.clone()),
        Ok(Reply {
            answer: "recovered".to_owned(),
            usage: Some(Usage {
                input_tokens: 3,
                output_tokens: 1,
                reasoning_tokens: None,
            }),
        }),
    ]);

    assert_eq!(complete(&model, request("one")).unwrap_err(), expected);
    assert_eq!(complete(&model, request("two")).unwrap().answer, "recovered");
    model.assert_exhausted();
}

#[test]
fn exhaustion() {
    let model = Scripted::reply("only");

    assert_eq!(complete(&model, request("one")).unwrap().answer, "only");
    assert_eq!(
        complete(&model, request("two")).unwrap_err(),
        Error::Backend("model script exhausted".to_owned())
    );
    model.assert_exhausted();
}

#[test]
fn request_recording() {
    let scripted = Scripted::reply("ok");
    let harness = Harness::new(scripted);
    let request = complete_request();

    assert_eq!(complete(&harness, request.clone()).unwrap().answer, "ok");
    assert_eq!(harness.requests(), vec![request.clone()]);

    let requests = harness.requests();
    let grants = mcp_grants(&requests[0]);
    assert_eq!(
        grants,
        vec![match &request.tools[1] {
            Tool::Mcp(grant) => grant,
            Tool::Function(_) => unreachable!(),
        }]
    );
}

#[test]
fn snapshot_sharing() {
    let harness = Harness::new(Scripted::answers(["one", "two"]));
    let other = harness.clone();

    let thread = std::thread::spawn(move || complete(&other, request("thread")));
    let main_answer = complete(&harness, request("main")).unwrap().answer;
    let thread_answer = thread.join().unwrap().unwrap().answer;
    let mut answers = [main_answer, thread_answer];
    answers.sort();
    assert_eq!(answers, ["one", "two"]);

    let mut messages = harness
        .requests()
        .into_iter()
        .map(|request| request.messages[0].content.clone())
        .collect::<Vec<_>>();
    messages.sort();
    assert_eq!(messages, ["main", "thread"]);
}

fn complete(model: &impl Model, request: Request) -> Result<Reply, Error> {
    block_on(model.create(request))
}

fn request(content: &str) -> Request {
    Request {
        messages: vec![Message {
            role: Role::User,
            content: content.to_owned(),
        }],
        ..Request::default()
    }
}

fn complete_request() -> Request {
    Request {
        model: Some("test-model".to_owned()),
        system: Some("system".to_owned()),
        messages: vec![
            Message {
                role: Role::System,
                content: "context".to_owned(),
            },
            Message {
                role: Role::User,
                content: "question".to_owned(),
            },
            Message {
                role: Role::Assistant,
                content: "prior".to_owned(),
            },
        ],
        generation: Some(Generation {
            temperature: Some(0.2),
            top_p: Some(0.9),
            max_tokens: Some(64),
            stop: vec!["done".to_owned()],
            seed: Some(7),
            effort: Some(Effort::High),
        }),
        format: Format::Schema(SchemaFormat {
            name: "result".to_owned(),
            schema: r#"{"type":"object"}"#.to_owned(),
        }),
        tools: vec![
            Tool::Function(Function {
                name: "lookup".to_owned(),
                description: "Look up a value".to_owned(),
                parameters: r#"{"type":"object"}"#.to_owned(),
            }),
            Tool::Mcp(McpGrant {
                name: "docs".to_owned(),
                tools: vec!["search".to_owned()],
                url: "https://mcp.example.test".to_owned(),
            }),
        ],
        references: Some("reference-guest".to_owned()),
        verify: vec!["cargo-check".to_owned()],
        lend_workspace: true,
    }
}
