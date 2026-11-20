//! The host refuses malformed requests before the backend runs, and the SDK
//! refuses one the WIT cannot carry. Each refusal is `invalid`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{Request, Tool, WorkerGrant};
use test_programs::{lookup, user, verdict_format};

omnia_sdk::command::export!(scenario);

async fn refused(request: Request) -> String {
    let error = request.complete().await.expect_err("the request is refused");
    assert_eq!(error.code(), "invalid", "unexpected: {error:?}");
    error.description().to_owned()
}

// a worker grant over the lent `.`, so the host's shape rules are what refuse it
async fn worker_refused(grant: WorkerGrant) -> String {
    refused(Request::builder().messages(vec![user("hi")]).workspace(".").worker(grant).build())
        .await
}

fn egress(entry: &str) -> WorkerGrant {
    WorkerGrant {
        egress: vec![entry.to_owned()],
        ..WorkerGrant::default()
    }
}

fn read(entry: &str) -> WorkerGrant {
    WorkerGrant {
        read: vec![entry.to_owned()],
        ..WorkerGrant::default()
    }
}

fn tool(name: &str, input_schema: &str) -> Tool {
    Tool {
        name: name.to_owned(),
        description: Some("a test tool".to_owned()),
        input_schema: input_schema.to_owned(),
    }
}

async fn scenario() {
    let empty = Request::builder().messages(vec![]).build();
    assert_eq!(refused(empty).await, "empty request");

    let blank = Request::builder().messages(vec![user("   ")]).build();
    assert_eq!(refused(blank).await, "empty request");

    let reserved =
        Request::builder().messages(vec![user("hi")]).tools(vec![tool("read", "{}")]).build();
    assert!(refused(reserved).await.contains("reserved tool name"));

    let repeated =
        Request::builder().messages(vec![user("hi")]).tools(vec![lookup(), lookup()]).build();
    assert!(refused(repeated).await.contains("duplicate tool name: lookup"));

    let bad_schema = Request::builder()
        .messages(vec![user("hi")])
        .tools(vec![tool("lookup", "not json")])
        .build();
    assert!(refused(bad_schema).await.contains("`lookup`"));

    let unparseable_schema =
        Request::builder().messages(vec![user("hi")]).format(verdict_format("not json")).build();
    assert!(refused(unparseable_schema).await.contains("not valid JSON"));

    // a worker grant nests in its lend, so the SDK refuses one without
    let unlent =
        Request::builder().messages(vec![user("hi")]).worker(egress("registry.npmjs.org")).build();
    assert_eq!(refused(unlent).await, "a worker grant requires a workspace lend");

    for (entry, what) in [
        ("10.0.0.1", "is an IP literal"),
        ("[::1]", "is an IP literal"),
        ("registry.npmjs.org:443", "carries a port"),
        ("https://registry.npmjs.org", "carries a scheme"),
        ("registry.npmjs.org/path", "carries a path"),
        ("*", "is not a host name"),
        ("bad_host", "is not a host name"),
        ("", "is not a host name"),
    ] {
        let detail = worker_refused(egress(entry)).await;
        assert!(detail.contains(what), "egress `{entry}`: {detail}");
    }

    for (entry, what) in [
        ("/opt/toolchains", "must be spelled `~/<path>`"),
        (".rustup", "must be spelled `~/<path>`"),
        ("~", "must be spelled `~/<path>`"),
        ("~/", "names the home itself"),
        ("~/.", "names the home itself"),
        ("~/./", "names the home itself"),
        ("~//", "names the home itself"),
        ("~/tools/../..", "holds `..`"),
        ("~/..", "holds `..`"),
        ("~//etc", "is not a plain `/`-separated path"),
        ("~/tools/", "is not a plain `/`-separated path"),
        ("~/./tools", "is not a plain `/`-separated path"),
        ("~/tools\\..\\..", "is not a plain `/`-separated path"),
    ] {
        let detail = worker_refused(read(entry)).await;
        assert!(detail.contains(what), "read root `{entry}`: {detail}");
    }

    for name in ["1ST", "A=B", "", "RUST-LOG"] {
        let grant = WorkerGrant {
            env: vec![(name.to_owned(), "x".to_owned())],
            ..WorkerGrant::default()
        };
        let detail = worker_refused(grant).await;
        assert!(detail.contains("is not an environment variable name"), "env `{name}`: {detail}");
    }
}
