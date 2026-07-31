//! Outbound `wasi:http` seam: the guest's outbound request crosses the WIT
//! boundary into the default backend's reqwest hook and reaches a real HTTP
//! server; the origin's response (and any failure) crosses back.
//!
//! The conformance guest's `/proxy` route performs the outbound request
//! described by the JSON body and reports the outcome, so each test asserts
//! both what the origin observed (wiremock) and what the guest saw.

use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use wiremock::matchers::{body_string, header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::fixture;

/// Drive the conformance guest's `/proxy` route and parse its JSON report.
async fn proxy(payload: &Value) -> Result<Value> {
    let fx = fixture::conformance().await?;
    let response =
        omnia_testkit::http::post_json(&fx.runtime, "/proxy", payload.to_string()).await?;
    ensure!(response.status().is_success(), "the proxy route responds");
    serde_json::from_slice(response.body()).context("parsing the proxy report")
}

#[test]
fn post_forwarded() -> Result<()> {
    fixture::RT.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string("test body"))
            .respond_with(ResponseTemplate::new(201).set_body_string("Created"))
            .mount(&server)
            .await;

        let report = proxy(&json!({
            "url": server.uri(),
            "method": "POST",
            "body": "test body",
        }))
        .await?;

        assert_eq!(report["status"], 201, "the origin's status crosses back: {report}");
        assert_eq!(report["body"], "Created", "the origin's body crosses back");

        let requests = server.received_requests().await.context("requests recorded")?;
        assert_eq!(requests.len(), 1);
        // The backend replaces any inherited Host header with the origin's:
        // exactly one, and it names the target.
        let hosts: Vec<_> = requests[0].headers.get_all("host").iter().collect();
        assert_eq!(hosts.len(), 1, "exactly one Host header reaches the origin");
        assert!(hosts[0].to_str()?.starts_with("127.0.0.1:"), "Host names the origin");

        Ok(())
    })
}

#[test]
fn headers_forwarded() -> Result<()> {
    fixture::RT.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("X-Custom-Header", "custom-value"))
            .and(header("Authorization", "Bearer token123"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let report = proxy(&json!({
            "url": server.uri(),
            "headers": [
                ["X-Custom-Header", "custom-value"],
                ["Authorization", "Bearer token123"],
            ],
        }))
        .await?;

        // The mock only matches when both guest headers reached the origin.
        assert_eq!(report["status"], 200, "{report}");
        Ok(())
    })
}

#[test]
fn forbidden_headers_stripped() -> Result<()> {
    fixture::RT.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("connection", "keep-alive")
                    .insert_header("transfer-encoding", "chunked")
                    .insert_header("upgrade", "websocket")
                    .insert_header("content-type", "application/json")
                    .insert_header("x-safe-header", "safe-value"),
            )
            .mount(&server)
            .await;

        let report = proxy(&json!({ "url": server.uri() })).await?;
        assert_eq!(report["status"], 200, "{report}");

        let names: Vec<&str> = report["headers"]
            .as_array()
            .context("headers reported")?
            .iter()
            .filter_map(|pair| pair[0].as_str())
            .collect();
        assert!(names.contains(&"content-type"), "permitted headers survive: {names:?}");
        assert!(names.contains(&"x-safe-header"), "permitted headers survive: {names:?}");
        for forbidden in ["connection", "transfer-encoding", "upgrade"] {
            assert!(!names.contains(&forbidden), "`{forbidden}` is stripped: {names:?}");
        }
        Ok(())
    })
}

#[test]
fn invalid_url_rejected() -> Result<()> {
    fixture::RT.block_on(async {
        let report = proxy(&json!({ "url": "not-a-valid-uri" })).await?;
        assert!(report["error"].is_string(), "a scheme-less URL fails the request: {report}");
        Ok(())
    })
}

#[test]
fn connection_refused() -> Result<()> {
    fixture::RT.block_on(async {
        // Bind then drop a listener so the port is known-dead, instead of
        // assuming a fixed port is unused.
        let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let report = proxy(&json!({ "url": format!("http://127.0.0.1:{port}/test") })).await?;
        assert!(report["error"].is_string(), "a refused connection surfaces: {report}");
        Ok(())
    })
}

#[test]
fn bad_client_cert_rejected() -> Result<()> {
    fixture::RT.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;

        let report = proxy(&json!({
            "url": server.uri(),
            "headers": [["Client-Cert", "not-valid-base64!!!"]],
        }))
        .await?;
        assert!(report["error"].is_string(), "undecodable base64 fails the request: {report}");

        // "invalid pem content" in base64: decodes, but is no certificate.
        let report = proxy(&json!({
            "url": server.uri(),
            "headers": [["Client-Cert", "aW52YWxpZCBwZW0gY29udGVudA=="]],
        }))
        .await?;
        assert!(report["error"].is_string(), "a non-PEM certificate fails the request: {report}");

        assert!(
            server.received_requests().await.context("requests recorded")?.is_empty(),
            "no request leaves the host with a bad certificate"
        );
        Ok(())
    })
}
