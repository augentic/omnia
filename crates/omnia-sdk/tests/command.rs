//! Command façade contracts: argv parsing, the failure envelope, completions,
//! the projector, the router's subcommands, and the exit boundary.

#![cfg(all(not(target_arch = "wasm32"), feature = "command"))]

mod support;

use std::borrow::Cow;
use std::fmt;
use std::future::{Ready, ready};

use clap::{Args, Parser, Subcommand};
use omnia_sdk::api::command::{
    Command, Failure, Format, IntoExit, Parsed, Render, Response, Shell, USAGE_EXIT, Verbosity,
    completions, parse,
};
use omnia_sdk::api::{Client, Context, DecodeError, Handlers, Metadata};
use omnia_sdk::{bad_request, not_found};
use serde::{Deserialize, Serialize};

use self::support::{Spans, fields};

#[derive(Debug, Parser)]
#[command(
    name = "app",
    bin_name = "app",
    version = "1.2.3",
    about = "Greets people",
    subcommand_required = true
)]
struct App {
    #[arg(long, default_value = "text", global = true)]
    format: Format,

    #[command(flatten)]
    verbosity: Verbosity,

    #[command(subcommand)]
    verb: Verb,
}

#[derive(Debug, Subcommand)]
enum Verb {
    Greet { name: String },
    Completions { shell: Shell },
}

/// Greet someone by name.
#[derive(Args, Debug, Deserialize)]
struct Greet {
    /// Who to greet
    name: String,
}

#[derive(Debug, Serialize)]
struct Greeting {
    text: String,
}

impl Render for Greeting {
    fn render(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        writeln!(out, "{}", self.text)
    }
}

#[derive(Args, Debug, Deserialize)]
struct Lookup {
    id: String,
}

#[test]
fn format_encode() {
    let greeting = Greeting {
        text: "hello, ada".to_string(),
    };

    let text = Format::Text.encode(&greeting);
    assert_eq!(text.media_type, "text/plain; charset=utf-8");
    assert_eq!(text.bytes, b"hello, ada\n");

    let json = Format::Json.encode(&greeting);
    assert_eq!(json.media_type, "application/json");
    let json = String::from_utf8(json.bytes).expect("json is utf-8");
    assert!(json.ends_with('\n'));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).expect("decode json"),
        serde_json::json!({ "text": "hello, ada" })
    );
}

#[test]
fn parse_app() {
    let Parsed::App(app) = parse::<App>(["app", "--format", "json", "greet", "ada"]) else {
        panic!("argv parses into the grammar");
    };

    assert_eq!(app.format, Format::Json);
    assert_eq!(app.verbosity, Verbosity::default());
    assert!(matches!(app.verb, Verb::Greet { name } if name == "ada"));
}

// The flags are global: they parse before and after the verb, every
// repetition counts, and the pair is refused.
#[test]
fn parse_verbosity() {
    let Parsed::App(verbose) = parse::<App>(["app", "-vv", "greet", "ada"]) else {
        panic!("`-vv` parses");
    };
    assert_eq!(verbose.verbosity, Verbosity { verbose: 2, quiet: 0 });

    let Parsed::App(quiet) = parse::<App>(["app", "greet", "ada", "--quiet", "-q"]) else {
        panic!("`--quiet -q` parses after the verb");
    };
    assert_eq!(quiet.verbosity, Verbosity { verbose: 0, quiet: 2 });

    let Parsed::Usage(error) = parse::<App>(["app", "greet", "ada", "-v", "-q"]) else {
        panic!("`-v` beside `-q` is a usage error");
    };
    assert_eq!(Response::usage(&error).exit, USAGE_EXIT);
}

#[test]
fn help_display() {
    let Parsed::Display(text) = parse::<App>(["app", "--help"]) else {
        panic!("help is a display outcome");
    };

    assert!(text.contains("Usage: app"));
    assert!(text.contains("greet"));
    assert!(text.contains("-v, --verbose"), "{text}");
    assert!(text.contains("-q, --quiet"), "{text}");

    // The flattened `Verbosity` docs stay out of the grammar's own about text.
    assert!(text.starts_with("Greets people"), "{text}");
    assert!(<App as clap::CommandFactory>::command().get_long_about().is_none());
}

#[test]
fn version_display() {
    let Parsed::Display(text) = parse::<App>(["app", "--version"]) else {
        panic!("version is a display outcome");
    };

    assert_eq!(text.trim(), "app 1.2.3");
}

#[test]
fn usage_error() {
    let Parsed::Usage(error) = parse::<App>(["app", "bogus"]) else {
        panic!("an unknown verb is a usage error");
    };
    let response = Response::usage(&error);

    assert_eq!(response.exit, USAGE_EXIT);
    assert_eq!(response.exit, 64);
    assert_eq!(response.stdout, b"");
    let stderr = String::from_utf8(response.stderr).expect("clap renders utf-8");
    assert!(stderr.contains("bogus"));
    assert!(stderr.contains("Usage: app"));
}

#[test]
fn failure_json() {
    let plain = Failure::from(not_found!("no item 42"));
    assert_eq!(plain.exit_code(), 2);
    assert_eq!(plain.hint(), None);
    assert_eq!(
        serde_json::to_value(&plain).expect("envelope serializes"),
        serde_json::json!({ "error": "not_found", "message": "no item 42", "exit-code": 2 })
    );

    let hinted = Failure::from(bad_request!("name is required")).with_hint("pass --name");
    assert_eq!(hinted.body().error, "bad_request");
    assert_eq!(
        serde_json::to_value(&hinted).expect("envelope serializes"),
        serde_json::json!({
            "error": "bad_request",
            "message": "name is required",
            "exit-code": 1,
            "hint": "pass --name"
        })
    );
}

#[test]
fn failure_text() {
    let failure = Failure::from(not_found!("no item 42")).with_hint("run `app list`");

    let mut text = String::new();
    failure.render(&mut text).expect("a String sink never fails");
    assert_eq!(text, "error[not_found]: no item 42\nhint: run `app list`\n");

    let encoded = Format::Text.encode(&failure);
    assert_eq!(encoded.bytes, text.into_bytes());
}

#[test]
fn failure_from_anyhow() {
    let foreign = Failure::from(anyhow::anyhow!("disk on fire"));
    assert_eq!(foreign.error().code(), "server_error");
    assert_eq!(foreign.exit_code(), 3);

    let domain = Failure::from(anyhow::Error::from(not_found!("gone")));
    assert_eq!(domain.exit_code(), 2);
}

#[test]
fn completions_bash() {
    let response = completions::<App>(Shell::Bash, "app");

    assert_eq!(response.exit, 0);
    assert_eq!(response.stderr, b"");
    let script = String::from_utf8(response.stdout).expect("completion script is utf-8");
    assert!(script.contains("_app"));
    assert!(script.contains("greet"));
    assert!(script.contains("--verbose"), "the flattened verbosity flags complete");
    assert!(script.contains("--quiet"));
}

#[test]
fn into_exit() {
    assert_eq!(Response::success("").into_exit(), Ok(()));
    assert_eq!(Response::failure("", 2).into_exit(), Err(2));
    assert_eq!(Response::failure("", USAGE_EXIT).into_exit(), Err(64));
}

#[test]
fn from_env_mints() {
    let metadata = Metadata::from_env("FROM_ENV_MINTS");

    let request_id = metadata.request_id.as_deref().expect("request id is minted");
    assert_eq!(request_id.len(), 32);
    assert!(request_id.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')));
    assert_eq!(metadata.correlation_id, metadata.request_id);
    assert_eq!(metadata.causation_id, None);
    assert_eq!(metadata.tenant, None);
    assert_ne!(Metadata::from_env("FROM_ENV_MINTS").request_id, metadata.request_id);
}

fn greet(_context: Context<()>, input: Greet) -> Ready<Result<Greeting, omnia_sdk::Error>> {
    let Greet { name } = input;
    ready(Ok(Greeting {
        text: format!("hello, {name}"),
    }))
}

fn lookup(_context: Context<()>, input: Lookup) -> Ready<Result<Greeting, omnia_sdk::Error>> {
    let Lookup { id } = input;
    ready(Err(not_found!("no item {id}")))
}

fn reject(_context: Context<()>, _input: Greet) -> Ready<Result<Greeting, omnia_sdk::Error>> {
    ready(Err(bad_request!("name is required")))
}

fn ping(_context: Context<()>, (): ()) -> Ready<Result<Greeting, omnia_sdk::Error>> {
    ready(Ok(Greeting {
        text: "pong".to_string(),
    }))
}

fn ada() -> Greet {
    Greet {
        name: "ada".to_string(),
    }
}

fn item(id: &str) -> Lookup {
    Lookup { id: id.to_string() }
}

fn stderr_json(response: &Response) -> serde_json::Value {
    serde_json::from_slice(&response.stderr).expect("stderr carries the JSON envelope")
}

#[tokio::test]
async fn call_text() {
    let client = Client::new(());
    let metadata = Metadata::default();
    let response = Command::new(&client, &metadata, Format::Text).call(greet, ada()).await;

    assert_eq!(response.exit, 0);
    assert_eq!(response.stderr, b"");
    assert_eq!(response.stdout, b"hello, ada\n");
}

#[tokio::test]
async fn call_json() {
    let client = Client::new(());
    let metadata = Metadata::default();
    let response = Command::new(&client, &metadata, Format::Json).call(greet, ada()).await;

    assert_eq!(response.exit, 0);
    let stdout = String::from_utf8(response.stdout).expect("json is utf-8");
    assert!(stdout.ends_with("}\n"));
    assert!(stdout.contains("\n  \"text\""));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout).expect("decode stdout"),
        serde_json::json!({ "text": "hello, ada" })
    );
}

#[tokio::test]
async fn call_handler_error() {
    let client = Client::new(());
    let metadata = Metadata::default();
    let command = Command::new(&client, &metadata, Format::Json);

    let response = command.call(lookup, item("42")).await;

    assert_eq!(response.exit, 2);
    assert_eq!(response.stdout, b"");
    assert_eq!(
        stderr_json(&response),
        serde_json::json!({ "error": "not_found", "message": "no item 42", "exit-code": 2 })
    );

    let text = Command::new(&client, &metadata, Format::Text).call(lookup, item("42")).await;
    assert_eq!(text.exit, 2);
    assert_eq!(text.stderr, b"error[not_found]: no item 42\n");
}

#[tokio::test]
async fn hints_by_error() {
    let client = Client::new(());
    let metadata = Metadata::default();
    let command = Command::new(&client, &metadata, Format::Text).hints(|error| {
        (error.code() == "not_found").then_some(Cow::Borrowed("run `app list` to see the items"))
    });

    let hinted = command.call(lookup, item("42")).await;
    assert_eq!(hinted.exit, 2);
    assert_eq!(
        hinted.stderr,
        b"error[not_found]: no item 42\nhint: run `app list` to see the items\n"
    );

    let unhinted = command.call(reject, ada()).await;
    assert_eq!(unhinted.exit, 1);
    assert_eq!(unhinted.stderr, b"error[bad_request]: name is required\n");
}

#[tokio::test]
async fn merged_commands() {
    let greeting = Handlers::new().command("greet", greet);
    let looking = Handlers::new().command("lookup", lookup);
    let pinging = Handlers::new().command("ping", ping);
    let router = Handlers::from([greeting, looking, pinging])
        .with_state(Client::new(()))
        .name("app")
        .version("1.2.3")
        .about("Greets people")
        .hints(|error| (error.code() == "not_found").then_some(Cow::Borrowed("run `app list`")));

    assert_eq!(router.commands().collect::<Vec<_>>(), ["greet", "lookup", "ping"]);

    let ok = router.run(["app", "greet", "ada"]).await;
    assert_eq!(ok.exit, 0);
    assert_eq!(ok.stdout, b"hello, ada\n");

    let unit = router.run(["app", "--format", "json", "ping"]).await;
    assert_eq!(unit.exit, 0);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&unit.stdout).expect("decode json"),
        serde_json::json!({ "text": "pong" })
    );

    let missing = router.run(["app", "lookup", "42"]).await;
    assert_eq!(missing.exit, 2);
    assert_eq!(missing.stderr, b"error[not_found]: no item 42\nhint: run `app list`\n");

    let unknown = router.run(["app", "bogus"]).await;
    assert_eq!(unknown.exit, USAGE_EXIT);

    let conflict = router.run(["app", "-v", "-q", "ping"]).await;
    assert_eq!(conflict.exit, USAGE_EXIT);

    let version = router.run(["app", "--version"]).await;
    assert_eq!(version.exit, 0);
    assert_eq!(String::from_utf8(version.stdout).expect("utf-8").trim(), "app 1.2.3");

    let completion = router.completions(Shell::Bash);
    assert_eq!(completion.exit, 0);
    let script = String::from_utf8(completion.stdout).expect("utf-8");
    assert!(script.contains("greet"));
    assert!(script.contains("lookup"));
}

#[tokio::test]
async fn command_span() {
    let (spans, _guard) = Spans::install();
    let router = Handlers::new().command("greet", greet).with_state(Client::new(())).name("app");
    let metadata = Metadata {
        request_id: Some("req-1".to_string()),
        correlation_id: Some("corr-1".to_string()),
        ..Metadata::default()
    };

    let response = router.run_with(["app", "greet", "ada"], &metadata).await;

    assert_eq!(response.exit, 0);
    assert_eq!(
        spans.handler(),
        fields([("request_id", "req-1"), ("correlation_id", "corr-1"), ("command", "greet")])
    );
}

// The input type's doc comments are the help text: the struct's for the
// subcommand's `about`, each field's for its argument.
#[tokio::test]
async fn help_from_docs() {
    let router = Handlers::new()
        .command("greet", greet)
        .with_state(Client::new(()))
        .name("app")
        .about("Greets people");

    let help = router.run(["app", "--help"]).await;
    assert_eq!(help.exit, 0);
    let text = String::from_utf8(help.stdout).expect("utf-8");
    assert!(text.starts_with("Greets people"), "{text}");
    assert!(text.contains("greet  Greet someone by name"), "{text}");

    let verb = router.run(["app", "greet", "-h"]).await;
    assert_eq!(verb.exit, 0);
    let text = String::from_utf8(verb.stdout).expect("utf-8");
    assert!(text.starts_with("Greet someone by name"), "{text}");
    assert!(text.contains("<NAME>  Who to greet"), "{text}");
}

// A decoder's refusal is a `bad_request` envelope, as a malformed payload
// is on every transport, not a clap usage error.
#[tokio::test]
async fn command_with_decoder() {
    let router = Handlers::new()
        .command_with(
            clap::Command::new("lookup").arg(clap::Arg::new("id").required(true)),
            lookup,
            |matches| {
                let id = matches.get_one::<String>("id").expect("id is required");
                if id.chars().all(|c| c.is_ascii_digit()) {
                    Ok(Lookup { id: id.clone() })
                } else {
                    Err(DecodeError::new(format!("`{id}` is not an item id")))
                }
            },
        )
        .with_state(Client::new(()));

    let decoded = router.run(["app", "lookup", "42"]).await;
    assert_eq!(decoded.exit, 2);
    assert_eq!(decoded.stderr, b"error[not_found]: no item 42\n");

    let refused = router.run(["app", "--format", "json", "lookup", "forty-two"]).await;
    assert_eq!(refused.exit, 1);
    assert_eq!(
        stderr_json(&refused),
        serde_json::json!({
            "error": "bad_request",
            "message": "`forty-two` is not an item id",
            "exit-code": 1
        })
    );
}

#[test]
#[should_panic(expected = "duplicate command `greet`")]
fn duplicate_command() {
    let _handlers = Handlers::<()>::new().command("greet", greet).command("greet", greet);
}

#[cfg(feature = "http")]
mod http_parity {
    use axum::Json;
    use axum::body::{Body, to_bytes};
    use axum::extract::Query;
    use axum::routing::get;
    use http::{Request, StatusCode};
    use omnia_sdk::api::ErrorBody;
    use tower::ServiceExt as _;

    use super::*;

    async fn lookup_route(
        context: Context<()>, Query(input): Query<Lookup>,
    ) -> Result<Json<Greeting>, omnia_sdk::Error> {
        lookup(context, input).await.map(Json)
    }

    // One `Handlers` carries the path and the subcommand, and the one router
    // answers both with the same error body.
    #[tokio::test]
    async fn handler_error() {
        let router = Handlers::new()
            .route("/items", get(lookup_route))
            .command("lookup", lookup)
            .with_state(Client::new(()));

        let response = router.run(["app", "--format", "json", "lookup", "42"]).await;
        let command_body: ErrorBody =
            serde_json::from_slice(&response.stderr).expect("stderr carries an error body");

        let request =
            Request::builder().uri("/items?id=42").body(Body::empty()).expect("build request");
        let http = router.oneshot(request).await.expect("router serves request");
        assert_eq!(http.status(), StatusCode::NOT_FOUND);
        let bytes = to_bytes(http.into_body(), usize::MAX).await.expect("collect body");
        let http_body: ErrorBody =
            serde_json::from_slice(&bytes).expect("http carries an error body");

        assert_eq!(command_body, http_body);
        assert_eq!(command_body.error, "not_found");
        assert_eq!(response.exit, 2);
    }
}
