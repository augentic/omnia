//! `wasi:http` seam: a request crosses into the guest and a response returns,
//! exercising `Request::from_http` / `Response::into_http` and the trigger
//! router without a TCP socket.

use anyhow::Result;
use omnia_testkit::http;

use crate::fixture;

#[test]
fn echo() -> Result<()> {
    fixture::RT.block_on(async {
        let fx = fixture::conformance().await?;

        let response = http::post_json(&fx.runtime, "/echo", r#"{"ping":"pong"}"#).await?;
        assert!(response.status().is_success(), "guest handles the request across the boundary");

        let body: serde_json::Value = serde_json::from_slice(response.body())?;
        assert_eq!(
            body["request"],
            serde_json::json!({ "ping": "pong" }),
            "the guest echoes the request body back across the boundary"
        );

        Ok(())
    })
}
