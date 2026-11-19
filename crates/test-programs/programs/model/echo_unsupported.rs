//! The echo default refuses what it cannot honour: `format::schema`, a
//! generation control, and an MCP server are each `unsupported`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::model::{
    Error, Format, Generation, McpGrant, Model as _, Request, SchemaFormat, Tool,
};
use test_programs::{VERDICT_SCHEMA, user};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let schema = Request::builder()
        .messages(vec![user("hi")])
        .format(Format::Schema(
            SchemaFormat::builder().name("verdict").schema(VERDICT_SCHEMA).build(),
        ))
        .build();
    let generation = Request::builder()
        .messages(vec![user("hi")])
        .generation(Generation::builder().temperature(0.25).build())
        .build();
    let server = Request::builder()
        .messages(vec![user("hi")])
        .tools(vec![Tool::Mcp(McpGrant::builder().name("docs").url("https://mcp.example").build())])
        .build();

    for (request, needle) in [
        (schema, "cannot satisfy format::schema"),
        (generation, "ignores generation controls"),
        (server, "reaches no MCP server"),
    ] {
        let error = WasiProvider.complete(request).await.expect_err("the echo refuses");
        assert!(
            matches!(error, Error::Backend(ref detail) if detail.starts_with("unsupported: ") && detail.contains(needle)),
            "unexpected: {error:?}"
        );
    }
}
