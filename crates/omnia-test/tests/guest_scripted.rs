//! The scripted pair at the handler rung: `Scripted` as a `Model`,
//! `ScriptedLoader` as a `Plugins` loader.

#![cfg(not(target_arch = "wasm32"))]

use std::panic::{AssertUnwindSafe, catch_unwind};

use omnia_sdk::model::{
    Error, Format, Function, Message, Model, Request, Role, SchemaFormat, Tool, ToolCall,
};
use omnia_sdk::plugins::{self, Digest, Location, Plugins};
use omnia_test::guest::{Scripted, ScriptedLoader, function_tools};
use omnia_test::{Exchange, SeenFormat};

fn user(content: &str) -> Request {
    Request::builder()
        .messages(vec![Message {
            role: Role::User,
            content: content.to_owned(),
        }])
        .build()
}

fn call(name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_owned(),
        arguments: arguments.to_owned(),
    }
}

fn digest(fill: &str) -> Digest {
    format!("sha256:{}", fill.repeat(64 / fill.len())).parse().expect("digest")
}

#[tokio::test]
async fn answers_in_order() {
    let model = Scripted::answering(["first", "second"]);
    assert_eq!(model.complete(user("a")).await.expect("reply").answer, "first");
    assert_eq!(model.complete(user("b")).await.expect("reply").answer, "second");

    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].messages[0].content, "b");
    let seen = model.seen();
    assert_eq!(seen[0].messages, ["a"]);
    model.assert_exhausted();
}

#[tokio::test]
async fn scripted_failures() {
    let model = Scripted::new([Err(Error::BudgetExhausted("cap".into()))]);
    assert_eq!(model.complete(user("a")).await, Err(Error::BudgetExhausted("cap".into())));
}

#[tokio::test]
async fn complete_with_calls() {
    let model = Scripted::answering(["done"])
        .calling(0, [call("lookup", r#"{"id":1}"#), call("write", "{}")]);

    let reply = model
        .complete_with(user("go"), |call: ToolCall| async move {
            if call.name == "lookup" { Ok("found".to_owned()) } else { Err("denied".to_owned()) }
        })
        .await
        .expect("reply");

    assert_eq!(reply.answer, "done");
    assert_eq!(
        model.exchanges(),
        [
            Exchange {
                tool: "lookup".into(),
                arguments: r#"{"id":1}"#.into(),
                outcome: Ok("found".into()),
            },
            Exchange {
                tool: "write".into(),
                arguments: "{}".into(),
                outcome: Err("denied".into()),
            },
        ]
    );
}

fn checked(content: &str) -> Request {
    Request::builder()
        .messages(vec![Message {
            role: Role::User,
            content: content.to_owned(),
        }])
        .check(true)
        .build()
}

// The handler: accepts a candidate containing "ok", otherwise corrects.
async fn judge(call: ToolCall) -> Result<String, String> {
    assert_eq!(call.name, "check");
    if call.arguments.contains("ok") {
        Ok(String::new())
    } else {
        Err(format!("no: {}", call.arguments))
    }
}

#[tokio::test]
async fn check_accepts() {
    let model = Scripted::answering(["ok"]);
    let reply = model.complete_with(checked("go"), judge).await.expect("reply");
    assert_eq!(reply.answer, "ok");
    assert_eq!(
        model.exchanges(),
        [Exchange {
            tool: "check".into(),
            arguments: "ok".into(),
            outcome: Ok(String::new()),
        }]
    );
    model.assert_exhausted();
}

#[tokio::test]
async fn check_corrects() {
    let model = Scripted::answering(["bad", "ok"]);
    let reply = model.complete_with(checked("go"), judge).await.expect("reply");
    assert_eq!(reply.answer, "ok");
    let exchanges = model.exchanges();
    assert_eq!(exchanges.len(), 2);
    assert_eq!(exchanges[0].outcome, Err("no: bad".into()));
    assert_eq!(exchanges[1].outcome, Ok(String::new()));
    assert_eq!(model.seen().len(), 2, "one scripted turn per attempt");
    model.assert_exhausted();
}

#[tokio::test]
async fn check_exhausts() {
    let model = Scripted::answering(["bad", "worse"]);
    let error = model.complete_with(checked("go"), judge).await.expect_err("never accepted");
    assert_eq!(error, Error::BudgetExhausted("no: worse".into()));
    model.assert_exhausted();
}

#[tokio::test]
async fn check_skips_failed() {
    let model = Scripted::new([Err(Error::Backend("offline".into()))]);
    let error = model.complete_with(checked("go"), judge).await.expect_err("backend failed");
    assert_eq!(error, Error::Backend("offline".into()));
    assert!(model.exchanges().is_empty(), "a failed turn has no candidate to check");
}

#[test]
fn complete_rejects_check() {
    let model = Scripted::default();
    let result = catch_unwind(AssertUnwindSafe(|| drop(model.complete(checked("a")))));
    assert!(result.is_err(), "complete must refuse a check request");
}

#[tokio::test]
async fn complete_rejects_calls() {
    let model = Scripted::answering(["x"]).calling(0, [call("t", "{}")]);
    let result = catch_unwind(AssertUnwindSafe(|| model.complete(user("a"))));
    assert!(result.is_err(), "complete must refuse scripted tool calls");
}

#[tokio::test]
async fn then_answers() {
    let model = Scripted::answering(["one"]).then(|| Err(Error::Backend("offline".into())));
    assert_eq!(model.complete(user("a")).await.expect("reply").answer, "one");
    assert_eq!(model.complete(user("b")).await, Err(Error::Backend("offline".into())));
}

#[test]
fn unscripted() {
    let model = Scripted::default();
    let result = catch_unwind(AssertUnwindSafe(|| drop(model.complete(user("a")))));
    assert!(result.is_err(), "an empty script panics on first use");
}

#[tokio::test]
async fn seen() {
    let model = Scripted::answering(["{}"]);
    let request = Request::builder()
        .system("be terse")
        .messages(vec![Message {
            role: Role::User,
            content: "hi".into(),
        }])
        .format(Format::Schema(SchemaFormat::builder().name("out").schema("{}").build()))
        .tools(vec![Tool::Function(
            Function::builder().name("lookup").description("d").parameters("{}").build(),
        )])
        .workspace(".")
        .build();
    model.complete(request).await.expect("reply");

    let seen = model.seen().remove(0);
    assert_eq!(seen.system.as_deref(), Some("be terse"));
    assert_eq!(seen.messages, ["hi"]);
    assert_eq!(
        seen.format,
        SeenFormat::Schema {
            name: "out".into(),
            schema: "{}".into()
        }
    );
    assert_eq!(seen.tools, ["lookup"]);
    assert_eq!(seen.workspace.as_deref(), Some("."));
    assert_eq!(function_tools(&model.requests()[0])[0].name, "lookup");
}

fn declared(name: &str) -> Location {
    Location::Declared(name.to_owned())
}

#[tokio::test]
async fn loader_digest() {
    let loader = ScriptedLoader::default().declare("tool").digest("tool", digest("ab"));
    let plugin = loader.load(&declared("tool"), None).await.expect("loads");
    assert_eq!(plugin.id(), "tool");
    assert_eq!(plugin.digest(), &digest("ab"));
    assert_eq!(loader.loads(), [(declared("tool"), None)]);
}

// A path or a package nothing scripts still loads, under a placeholder
// digest that is stable per name and distinct across names; the name is
// the one the location registers under.
#[tokio::test]
async fn loader_placeholder() {
    let loader = ScriptedLoader::default();
    let path = Location::Path("./adapters/other.wasm".to_owned());
    let first = loader.load(&path, None).await.expect("loads");
    assert_eq!(first.id(), "other", "a path registers as its file stem");
    let second = loader.load(&path, None).await.expect("loads");
    assert_eq!(first.digest(), second.digest(), "placeholder digests are deterministic");
    let package = Location::Registry {
        package: "acme:another@1.0.0".to_owned(),
        endpoint: None,
    };
    let third = loader.load(&package, None).await.expect("loads");
    assert_eq!(third.id(), "acme:another", "a package registers as its reference, unversioned");
    assert_ne!(first.digest(), third.digest(), "placeholder digests are per name");
}

// A declared name loads only once declared, and never with a digest of its
// own: the deployment's entry carries the pin.
#[tokio::test]
async fn loader_declared() {
    let loader = ScriptedLoader::default().declare("tool");
    assert_eq!(loader.load(&declared("tool"), None).await.expect("loads").id(), "tool");
    let undeclared = loader.load(&declared("other"), None).await.expect_err("undeclared");
    assert!(
        matches!(&undeclared, plugins::Error::Refused(detail) if detail.contains("no guest `other`")),
        "{undeclared}"
    );
    let pinned = loader.load(&declared("tool"), Some(&digest("ab"))).await.expect_err("pinned");
    assert!(matches!(pinned, plugins::Error::Refused(_)), "{pinned}");
    assert_eq!(loader.loads().len(), 3, "every load is recorded, refused or not");
}

// A digest on the call is held against the resolved one, as the host holds
// a pin against the bytes.
#[tokio::test]
async fn loader_pin() {
    let loader = ScriptedLoader::default().digest("tool", digest("ab"));
    let path = Location::Path("tool.wasm".to_owned());
    let pinned = loader.load(&path, Some(&digest("ab"))).await.expect("the pin matches");
    assert_eq!(pinned.digest(), &digest("ab"));
    let mismatch = loader.load(&path, Some(&digest("ef"))).await.expect_err("the pin misses");
    assert!(
        matches!(&mismatch, plugins::Error::Refused(detail) if detail.contains("not its pinned digest")),
        "{mismatch}"
    );
    assert_eq!(loader.loads(), [(path.clone(), Some(digest("ab"))), (path, Some(digest("ef")))]);
}

// The default sits below the scripted digest: an unscripted load takes it
// in place of the placeholder, a scripted digest still wins.
#[tokio::test]
async fn loader_defaulting() {
    let loader = ScriptedLoader::default()
        .declare("other")
        .declare("tool")
        .digest("tool", digest("ab"))
        .defaulting(digest("ef"));
    assert_eq!(loader.load(&declared("other"), None).await.expect("loads").digest(), &digest("ef"));
    assert_eq!(loader.load(&declared("tool"), None).await.expect("loads").digest(), &digest("ab"));
    assert_eq!(loader.loads(), [(declared("other"), None), (declared("tool"), None)]);
}

#[tokio::test]
async fn loader_scripted_refusal_wins() {
    let loader = ScriptedLoader::default()
        .declare("tool")
        .digest("tool", digest("ab"))
        .refuse("tool", plugins::Error::Unavailable("registry down".into()));
    assert_eq!(
        loader.load(&declared("tool"), None).await,
        Err(plugins::Error::Unavailable("registry down".into()))
    );
}
