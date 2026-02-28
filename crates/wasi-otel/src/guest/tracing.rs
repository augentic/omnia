//! # Tracing

use std::sync::{Arc, Mutex};
use std::time::Duration;

use opentelemetry::{Context, global, trace as otel};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{SdkTracerProvider, Span, SpanData, SpanProcessor};

use crate::guest::generated::omnia::otel::tracing as wasi;

pub fn init(resource: Resource) -> SdkTracerProvider {
    let processor = Processor::new();
    let provider =
        SdkTracerProvider::builder().with_resource(resource).with_span_processor(processor).build();
    global::set_tracer_provider(provider.clone());
    provider
}

#[derive(Debug)]
struct Processor {
    resource: Resource,
    spans: Arc<Mutex<Vec<SpanData>>>,
}

impl Processor {
    #[must_use]
    fn new() -> Self {
        Self {
            resource: Resource::builder_empty().build(),
            spans: Arc::new(Mutex::new(Vec::new())),
        }
    }
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
        let spans =
            self.spans.lock().map_err(|e| OTelSdkError::InternalFailure(e.to_string()))?.to_vec();
        if spans.is_empty() {
            return Ok(());
        }

        wit_bindgen::spawn(async move {
            let spans = spans.into_iter().map(Into::into).collect::<Vec<_>>();
            if let Err(e) = wasi::export(spans).await {
                tracing::error!("failed to export spans: {e}");
            }
        });

        Ok(())
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.resource = resource.clone();
    }
}

impl From<wasi::SpanContext> for otel::SpanContext {
    fn from(value: wasi::SpanContext) -> Self {
        let trace_id = otel::TraceId::from_hex(&value.trace_id).unwrap_or(otel::TraceId::INVALID);
        let span_id = otel::SpanId::from_hex(&value.span_id).unwrap_or(otel::SpanId::INVALID);
        let trace_state = otel::TraceState::from_key_value(value.trace_state)
            .unwrap_or_else(|_| otel::TraceState::default());
        Self::new(trace_id, span_id, value.trace_flags.into(), value.is_remote, trace_state)
    }
}

impl From<wasi::TraceFlags> for otel::TraceFlags {
    fn from(value: wasi::TraceFlags) -> Self {
        if value.contains(wasi::TraceFlags::SAMPLED) { Self::SAMPLED } else { Self::default() }
    }
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
            trace_state: sc
                .trace_state()
                .header()
                .split(',')
                .filter_map(|s| {
                    if let Some((key, value)) = s.split_once('=') {
                        Some((key.to_string(), value.to_string()))
                    } else {
                        None
                    }
                })
                .collect(),
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
