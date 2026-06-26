//! # Tracing
//!
//! Tracing functionality.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use anyhow::Result;
use http::{Request, Response};
use opentelemetry::metrics::Histogram;
use opentelemetry::{KeyValue, global};
use tower::{Layer, Service};
use tracing::Instrument;

/// Layer that records request latency and emits OpenTelemetry metrics.
#[derive(Clone)]
pub struct TracingLayer;

impl Default for TracingLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl TracingLayer {
    /// Creates a new instance of `TracingLayer`.
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }
}

impl<S> Layer<S> for TracingLayer {
    type Service = TracingService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TracingService {
            inner,
            histogram: global::meter("http_server")
                .f64_histogram("http.server.duration")
                .with_unit("s")
                .build(),
        }
    }
}

/// Tracing service that wraps a tower service.
#[derive(Clone)]
pub struct TracingService<S> {
    inner: S,
    histogram: Histogram<f64>,
}

impl<S, Body> Service<Request<Body>> for TracingService<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Send + 'static,
    S::Future: Send + 'static,
    S::Error: std::fmt::Debug,
{
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    type Response = S::Response;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let uri = req.uri().to_string();
        let method = req.method().to_string();
        let route = req.uri().path().to_string();
        let span = tracing::info_span!(
            target: "api",
            "request",
            uri = %uri,
            otel.kind = "server",
            http.request.method = %method,
            http.target = %uri,
            http.route = %route,
            category = "http",
            instrumentation.provider = "opentelemetry"
        );
        let histogram = self.histogram.clone();

        let inner_fut = self.inner.call(req);

        let self_fut = async move {
            let start = Instant::now();
            let response = inner_fut.await;
            let duration = start.elapsed();
            let status = response.as_ref().map_or(500, |r| i64::from(r.status().as_u16()));

            histogram.record(
                duration.as_secs_f64(),
                &[
                    KeyValue::new("http.request.method", method.clone()),
                    KeyValue::new("http.response.status_code", status),
                    KeyValue::new("http.route", route.clone()),
                ],
            );

            let span = tracing::Span::current();
            span.record("http.response.status_code", status);

            response
        };

        Box::pin(self_fut.instrument(span))
    }
}
