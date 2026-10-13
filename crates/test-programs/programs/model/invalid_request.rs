//! The host refuses malformed requests before the backend runs.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::model::{
    Error, Format, Function, Model as _, Request, SchemaFormat, Tool, WasiModel, WorkerGrant,
};
use test_programs::user;

omnia_sdk::command!(scenario);

async fn refused(request: Request) -> String {
    match WasiModel.complete(request).await {
        Err(Error::InvalidRequest(detail)) => detail,
        other => panic!("expected invalid-request, got {other:?}"),
    }
}

fn verdict(schema: &str) -> Format {
    Format::Schema(SchemaFormat::builder().name("verdict").schema(schema).build())
}

// a worker grant over the lent `.`, so the host's shape rules are what refuse it
async fn worker_refused(grant: WorkerGrant) -> String {
    refused(Request::builder().messages(vec![user("hi")]).workspace(".").worker(grant).build())
        .await
}

fn egress(entry: &str) -> WorkerGrant {
    WorkerGrant::builder().egress(vec![entry.to_owned()]).build()
}

fn read(entry: &str) -> WorkerGrant {
    WorkerGrant::builder().read(vec![entry.to_owned()]).build()
}

async fn scenario() {
    let empty = Request::builder().messages(vec![]).build();
    assert_eq!(refused(empty).await, "empty request");

    let blank = Request::builder().messages(vec![user("   ")]).build();
    assert_eq!(refused(blank).await, "empty request");

    let reserved = Request::builder()
        .messages(vec![user("hi")])
        .tools(vec![Tool::Function(
            Function::builder()
                .name("read")
                .description("shadow a host-injected tool")
                .parameters("{}")
                .build(),
        )])
        .build();
    assert!(refused(reserved).await.contains("reserved tool name"));

    let check = Request::builder()
        .messages(vec![user("hi")])
        .tools(vec![Tool::Function(
            Function::builder()
                .name("check")
                .description("shadow the check callback")
                .parameters("{}")
                .build(),
        )])
        .build();
    assert!(refused(check).await.contains("reserved tool name"));

    let bad_parameters = Request::builder()
        .messages(vec![user("hi")])
        .tools(vec![Tool::Function(
            Function::builder()
                .name("lookup")
                .description("look something up")
                .parameters("not json")
                .build(),
        )])
        .build();
    assert!(refused(bad_parameters).await.contains("`lookup`"));

    let unparseable_schema =
        Request::builder().messages(vec![user("hi")]).format(verdict("not json")).build();
    assert!(refused(unparseable_schema).await.contains("not valid JSON"));

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
        let grant = WorkerGrant::builder().env(vec![(name.to_owned(), "x".to_owned())]).build();
        let detail = worker_refused(grant).await;
        assert!(detail.contains("is not an environment variable name"), "env `{name}`: {detail}");
    }
}
