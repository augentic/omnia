//! # Model example — session guest
//!
//! A `wasi:cli/command` reactor that **imports** `omnia:model/completion` and
//! drives one completion session when the host calls `wasi:cli/run`. It is
//! scenario-driven via its CLI argument — the seam suite
//! (`crates/seam-suite/tests/model.rs`) selects the behavior it needs —
//! making it the model acceptance vehicle.
//!
//! The default scenario is the happy path on the `omnia-guest` sugar: a
//! tool-less schema completion whose prompt is assembled with `Sections`,
//! lending the `.` mount through `grants.workspace` when the host preopened
//! one. The tool scenarios (`round_trip`, `budget`, `oversize`) answer the
//! model's calls with a `complete_with` closure over guest locals. The
//! misbehavior scenarios (`parallel`, `drops_session`, `closes_results`,
//! `stall`) use the raw session bindings, since the sugar cannot express
//! them.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{
    Format, Function, Message, Model as _, Request, Role, SchemaFormat, Tool, ToolCall, WasiModel,
};
use omnia_wasi_model::prompt::Sections;
use omnia_wasi_model::{completion, wit_stream};
use wasip3::exports::cli::run::Guest;
use wasip3::filesystem::preopens;

const SHELF: &[(&str, &str)] = &[("k1", "v1"), ("k2", "v2")];

struct CliGuest;

wasip3::cli::command::export!(CliGuest);

impl Guest for CliGuest {
    async fn run() -> Result<(), ()> {
        let arguments = wasip3::cli::environment::get_arguments();
        let scenario = arguments.get(1).map_or("default", String::as_str);
        let output = match scenario {
            "default" => default_completion().await,
            "round_trip" => round_trip().await,
            "budget" => echo_tool_loop().await,
            "oversize" => oversize_result().await,
            "parallel" => parallel().await,
            "drops_session" => drops_session().await,
            "closes_results" => closes_results().await,
            "stall" => stall().await,
            other => format!("unknown scenario: {other}"),
        };
        println!("{output}");
        Ok(())
    }
}

fn render(outcome: Result<omnia_guest::model::Reply, omnia_guest::model::Error>) -> String {
    match outcome {
        Ok(reply) => reply.answer,
        Err(error) => format!("error: {error:?}"),
    }
}

// The happy path: a tool-less schema completion over the sugar, lending the
// `.` mount when the host preopened one (with no mount the preopen table is
// empty and the guest lends nothing).
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

fn tool_request(name: &str, description: &str) -> Request {
    Request::builder()
        .messages(vec![Message {
            role: Role::User,
            content: "use the tool".to_owned(),
        }])
        .format(Format::Json)
        .tools(vec![Tool::Function(
            Function::builder()
                .name(name)
                .description(description)
                .parameters("{\"type\":\"object\"}")
                .build(),
        )])
        .build()
}

// One declared tool answered by a closure over guest locals (the shelf).
async fn round_trip() -> String {
    let request = tool_request("lookup", "look up a shelf value by key");
    let outcome = WasiModel
        .complete_with(request, |call: ToolCall| async move {
            SHELF
                .iter()
                .find(|(key, _)| *key == call.arguments)
                .map(|(_, value)| (*value).to_owned())
                .ok_or_else(|| format!("no shelf value for `{}`", call.arguments))
        })
        .await;
    render(outcome)
}

// Answers every call; the probe backend loops until the host's call budget
// trips, so the reply carries `budget-exhausted`.
async fn echo_tool_loop() -> String {
    let request = tool_request("echo", "echo the arguments back");
    let outcome =
        WasiModel.complete_with(request, |call: ToolCall| async move { Ok(call.arguments) }).await;
    render(outcome)
}

// Every answer blows the probe's result byte cap.
async fn oversize_result() -> String {
    let request = tool_request("blob", "return a large blob");
    let outcome = WasiModel
        .complete_with(request, |_call: ToolCall| async move { Ok("x".repeat(4096)) })
        .await;
    render(outcome)
}

// -- Raw-binding scenarios below: behaviors the sugar cannot express. --

fn wire_request(tool: &str) -> completion::Request<'static> {
    completion::Request {
        model: None,
        system: None,
        messages: vec![completion::Message {
            role: completion::Role::User,
            content: "use the tool".to_owned(),
        }],
        generation: None,
        format: completion::Format::Json,
        tools: vec![completion::Tool::Function(completion::Function {
            name: tool.to_owned(),
            description: "seam probe tool".to_owned(),
            parameters: "{\"type\":\"object\"}".to_owned(),
        })],
        grants: completion::Grants { workspace: None },
    }
}

fn reply_event(reply: Result<completion::Reply, completion::Error>) -> String {
    match reply {
        Ok(reply) => format!("reply-ok:{}", reply.answer),
        Err(error) => format!("reply-err:{error:?}"),
    }
}

// Reads a batch of three calls before answering any, then answers in reverse
// order: ids correlate and the results stream is unordered.
async fn parallel() -> String {
    let (mut results, results_rx) = wit_stream::new();
    let session = match completion::create(wire_request("echo"), results_rx).await {
        Ok(session) => session,
        Err(error) => return format!("error: {error:?}"),
    };
    let completion::Session { mut calls, reply } = session;
    let mut events = Vec::new();

    let mut batch = Vec::new();
    for _ in 0..3 {
        match calls.next().await {
            Some(call) => batch.push(call),
            None => return "calls closed before the batch completed".to_owned(),
        }
    }
    for call in batch.into_iter().rev() {
        let answered = format!("answered:{}", call.id);
        let result = completion::ToolResult {
            id: call.id,
            output: Ok(format!("echo:{}", call.arguments)),
        };
        match results.write_one(result).await {
            None => events.push(answered),
            Some(_) => events.push("write-rejected".to_owned()),
        }
    }

    while let Some(call) = calls.next().await {
        events.push(format!("unexpected-call:{}", call.id));
    }
    events.push("calls-closed".to_owned());
    events.push(reply_event(reply.await));
    events.join(";")
}

// Answers the first call, then drops the calls reader and the reply future
// mid-loop. Heartbeats on the results stream until the host acknowledges the
// session's end by rejecting a write, so the drops are observed while the
// instance is still live.
async fn drops_session() -> String {
    let (mut results, results_rx) = wit_stream::new();
    let session = match completion::create(wire_request("echo"), results_rx).await {
        Ok(session) => session,
        Err(error) => return format!("error: {error:?}"),
    };
    let completion::Session { mut calls, reply } = session;
    let mut events = Vec::new();

    let Some(call) = calls.next().await else {
        return "calls closed before the first call".to_owned();
    };
    let result = completion::ToolResult {
        id: call.id.clone(),
        output: Ok(format!("echo:{}", call.arguments)),
    };
    match results.write_one(result).await {
        None => events.push(format!("answered:{}", call.id)),
        Some(_) => events.push("write-rejected".to_owned()),
    }

    drop(calls);
    drop(reply);
    events.push("dropped-session".to_owned());

    loop {
        let heartbeat = completion::ToolResult {
            id: "heartbeat".to_owned(),
            output: Ok(String::new()),
        };
        if results.write_one(heartbeat).await.is_some() {
            events.push("host-acked-via-results-reject".to_owned());
            break;
        }
    }
    events.join(";")
}

// Receives one call and drops the results writer without answering: the
// pending call hard-fails host-side and the reply still resolves with a
// typed error.
async fn closes_results() -> String {
    let (results, results_rx) = wit_stream::new::<completion::ToolResult>();
    let session = match completion::create(wire_request("echo"), results_rx).await {
        Ok(session) => session,
        Err(error) => return format!("error: {error:?}"),
    };
    let completion::Session { mut calls, reply } = session;
    let mut events = Vec::new();

    let Some(call) = calls.next().await else {
        return "calls closed before the first call".to_owned();
    };
    events.push(format!("received:{}", call.id));
    drop(results);
    events.push("results-writer-dropped".to_owned());

    while let Some(call) = calls.next().await {
        events.push(format!("unexpected-call:{}", call.id));
    }
    events.push("calls-closed".to_owned());
    events.push(reply_event(reply.await));
    events.join(";")
}

// Receives one call and never answers it, keeping every session end open:
// the host's per-call timeout must end the session with a typed error.
async fn stall() -> String {
    let (_results, results_rx) = wit_stream::new::<completion::ToolResult>();
    let session = match completion::create(wire_request("echo"), results_rx).await {
        Ok(session) => session,
        Err(error) => return format!("error: {error:?}"),
    };
    let completion::Session { mut calls, reply } = session;
    let mut events = Vec::new();

    let Some(call) = calls.next().await else {
        return "calls closed before the first call".to_owned();
    };
    events.push(format!("received:{}", call.id));
    events.push(reply_event(reply.await));
    events.join(";")
}
