//! # OpenTelemetry Wasm Guest
//!
//! This module demonstrates comprehensive OpenTelemetry instrumentation in a
//! WebAssembly guest. It showcases both the `tracing` API and the native
//! OpenTelemetry API for distributed tracing and metrics.

#![cfg(target_arch = "wasm32")]

use axum::routing::{options, post};
use axum::{Json, Router};
use http::Method;
use opentelemetry::trace::{TraceContextExt, Tracer};
use opentelemetry::{KeyValue, global};
use qwasr_sdk::HttpResult;
use serde_json::{Value, json};
use tower_http::cors::{Any, CorsLayer};
use tracing::Level;
use wasip3::exports::http::handler::Guest;
use wasip3::http::types::{ErrorCode, Request, Response};

struct Http;
wasip3::http::service::export!(Http);

impl Guest for Http {
    /// Routes requests and demonstrates telemetry patterns.
    #[qwasr_wasi_otel::instrument(name = "http_guest_handle", level = Level::DEBUG)]
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        // tracing-based metrics
        tracing::info!(monotonic_counter.tracing_counter = 1, key1 = "value 1");
        tracing::info!(gauge.tracing_gauge = 1);

        // OpenTelemetry metrics API
        let meter = global::meter("my_meter");
        let counter = meter.u64_counter("otel_counter").build();
        counter.add(1, &[KeyValue::new("key1", "value 1")]);

        // OpenTelemetry spans
        let tracer = global::tracer("basic");
        tracer.in_span("main-operation", |cx| {
            let span = cx.span();
            span.set_attribute(KeyValue::new("my-attribute", "my-value"));
            span.add_event("main span event", vec![KeyValue::new("foo", "1")]);

            tracer.in_span("child-operation", |cx| {
                cx.span().add_event("sub span event", vec![KeyValue::new("bar", "1")]);
            });

            tracing::info_span!("child info span").in_scope(|| {
                tracing::info!("info event");
            });
        });

        tracing::info_span!("handler span")
            .in_scope(|| {
                tracing::info!("received request");

                let router = Router::new()
                    .layer(
                        CorsLayer::new()
                            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                            .allow_headers(Any)
                            .allow_origin(Any),
                    )
                    .route("/", post(handler))
                    .route("/", options(handle_options));

                qwasr_wasi_http::serve(router, request)
            })
            .await
    }
}

/// Simple JSON echo handler.
#[axum::debug_handler]
#[qwasr_wasi_otel::instrument]
async fn handler(Json(body): Json<Value>) -> HttpResult<Json<Value>> {
    tracing::info!("handling request: {:?}", body);
    Ok(Json(json!({
        "message": "Hello, World!",
        "request": body
    })))
}

/// Handles CORS preflight OPTIONS requests.
async fn handle_options() -> HttpResult<()> {
    Ok(())
}
