//! # Tracing

use std::sync::{Arc, Mutex};
use std::time::Duration;

use opentelemetry::{Context, trace as otel};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, Span, SpanData, SpanProcessor};

use crate::guest::generated::omnia::otel::tracing as wasi;

/// Ended spans awaiting export, shared between the processor and [`export`].
pub type SpanBuffer = Arc<Mutex<Vec<SpanData>>>;

pub fn init(resource: Resource) -> (SdkTracerProvider, SpanBuffer) {
    let spans = SpanBuffer::default();
    let processor = Processor {
        spans: Arc::clone(&spans),
    };
    let provider =
        SdkTracerProvider::builder().with_resource(resource).with_span_processor(processor).build();
    (provider, spans)
}

/// Export all buffered spans to the host.
pub async fn export(buffer: &SpanBuffer) {
    // Scoped rather than `drop`ped: a guard that was ever borrowed stays
    // live to the end of its scope for the coroutine, which would make this
    // future `!Send` across the `export` await.
    let spans = {
        let Ok(mut guard) = buffer.lock() else { return };
        std::mem::take(&mut *guard)
    };
    if spans.is_empty() {
        return;
    }

    let spans = spans.into_iter().map(Into::into).collect::<Vec<_>>();
    if let Err(e) = wasi::export(spans).await {
        tracing::error!("failed to export spans: {e}");
    }
}

#[derive(Debug)]
struct Processor {
    spans: SpanBuffer,
}

impl SpanProcessor for Processor {
    fn on_start(&self, _: &mut Span, _cx: &Context) {}

    fn on_end(&self, span: SpanData) {
        if !span.span_context.is_sampled() {
            return;
        }
        if let Ok(mut guard) = self.spans.lock() {
            guard.push(span);
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _: Duration) -> OTelSdkResult {
        Ok(())
    }

    fn set_resource(&mut self, _: &Resource) {}
}

impl From<SpanData> for wasi::SpanData {
    fn from(sd: SpanData) -> Self {
        Self {
            span_context: sd.span_context.into(),
            parent_span_id: sd.parent_span_id.to_string(),
            span_kind: sd.span_kind.into(),
            name: sd.name.to_string(),
            start_time: sd.start_time.into(),
            end_time: sd.end_time.into(),
            attributes: sd.attributes.into_iter().map(Into::into).collect(),
            events: sd.events.events.into_iter().map(Into::into).collect(),
            links: sd.links.links.into_iter().map(Into::into).collect(),
            status: sd.status.into(),
            instrumentation_scope: sd.instrumentation_scope.into(),
            dropped_attributes: sd.dropped_attributes_count,
            dropped_events: sd.events.dropped_count,
            dropped_links: sd.links.dropped_count,
        }
    }
}

impl From<otel::SpanContext> for wasi::SpanContext {
    fn from(sc: otel::SpanContext) -> Self {
        Self {
            trace_id: format!("{:x}", sc.trace_id()),
            span_id: format!("{:x}", sc.span_id()),
            trace_flags: sc.trace_flags().into(),
            is_remote: sc.is_remote(),
            trace_state: crate::trace_state::parse(&sc.trace_state().header()),
        }
    }
}

impl From<otel::TraceFlags> for wasi::TraceFlags {
    fn from(tf: otel::TraceFlags) -> Self {
        if tf.is_sampled() { Self::SAMPLED } else { Self::empty() }
    }
}

impl From<otel::SpanKind> for wasi::SpanKind {
    fn from(sk: otel::SpanKind) -> Self {
        match sk {
            otel::SpanKind::Client => Self::Client,
            otel::SpanKind::Server => Self::Server,
            otel::SpanKind::Producer => Self::Producer,
            otel::SpanKind::Consumer => Self::Consumer,
            otel::SpanKind::Internal => Self::Internal,
        }
    }
}

impl From<otel::Event> for wasi::Event {
    fn from(event: otel::Event) -> Self {
        Self {
            name: event.name.to_string(),
            time: event.timestamp.into(),
            attributes: event.attributes.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<otel::Link> for wasi::Link {
    fn from(link: otel::Link) -> Self {
        Self {
            span_context: link.span_context.into(),
            attributes: link.attributes.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<otel::Status> for wasi::Status {
    fn from(status: otel::Status) -> Self {
        match status {
            otel::Status::Unset => Self::Unset,
            otel::Status::Error { description } => Self::Error(description.to_string()),
            otel::Status::Ok => Self::Ok,
        }
    }
}
