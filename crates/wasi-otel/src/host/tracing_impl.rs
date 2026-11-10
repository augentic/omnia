//! # WASI Tracing

use std::sync::Once;

use anyhow::Result;
use omnia_core::HostView;
use opentelemetry::trace::{SpanId, TraceContextExt, TraceFlags};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::InstrumentationScope;
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::span::{Event, Link};
use opentelemetry_proto::tonic::trace::v1::status::StatusCode;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span, Status};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use wasmtime::component::Accessor;

use crate::host::generated::omnia::otel::tracing::{self as wasi, HostWithStore};
use crate::host::types_impl::{datetime_nanos, decode_id};
use crate::{OtelClient, WasiOtel};

pub fn resource_spans(
    spans: Vec<wasi::SpanData>, resource: &opentelemetry_sdk::Resource,
) -> Vec<ResourceSpans> {
    // Linear scan: an export carries a handful of scopes, and the generated types have no `Hash`.
    let mut scope_spans: Vec<ScopeSpans> = Vec::new();
    for span in spans {
        let schema_url = span.instrumentation_scope.schema_url.clone().unwrap_or_default();
        let scope = InstrumentationScope::from(span.instrumentation_scope.clone());
        if let Some(group) = scope_spans
            .iter_mut()
            .find(|group| group.scope.as_ref() == Some(&scope) && group.schema_url == schema_url)
        {
            group.spans.push(span.into());
        } else {
            scope_spans.push(ScopeSpans {
                scope: Some(scope),
                schema_url,
                spans: vec![span.into()],
            });
        }
    }

    vec![ResourceSpans {
        resource: Some(Resource {
            attributes: resource.iter().map(Into::into).collect(),
            dropped_attributes_count: 0,
            entity_refs: vec![],
        }),
        scope_spans,
        schema_url: resource.schema_url().map(Into::into).unwrap_or_default(),
    }]
}

impl<T> HostWithStore<T> for WasiOtel {
    async fn export(
        accessor: &Accessor<T, Self>, mut span_data: Vec<wasi::SpanData>,
    ) -> Result<(), wasi::Error> {
        let Some(resource) = omnia_core::telemetry::resource() else {
            tracing::warn!("otel resource not initialized, skipping trace export");
            return Ok(());
        };

        let ctx = tracing::Span::current().context();
        let parent_ctx = ctx.span().span_context().clone();
        if !parent_ctx.is_valid() {
            // once per process: a warning per export would drown the console
            static WARNED: Once = Once::new();
            WARNED.call_once(|| {
                tracing::warn!(
                    "no host span is live: guest spans are dropped until the guest runs inside \
                     an enabled `tracing` span (the runtime opens one per `wasi:cli/run` and \
                     per trigger request at INFO)"
                );
            });
            return Ok(());
        }

        let invalid_id = SpanId::INVALID.to_string();

        for sp in &mut span_data {
            sp.span_context.trace_id = parent_ctx.trace_id().to_string();

            // top-level spans need to be parented to the host span
            if sp.parent_span_id == invalid_id || sp.parent_span_id.is_empty() {
                sp.parent_span_id = parent_ctx.span_id().to_string();
            }
        }

        let resource_spans = resource_spans(span_data, resource);
        let export = ExportTraceServiceRequest { resource_spans };

        accessor.with(|mut store| store.get().client.export_traces(export)).await?;

        Ok(())
    }
}

impl wasi::Host for HostView<'_, dyn OtelClient> {}

impl From<wasi::SpanData> for Span {
    fn from(span: wasi::SpanData) -> Self {
        Self {
            trace_id: decode_id(&span.span_context.trace_id),
            span_id: decode_id(&span.span_context.span_id),
            trace_state: crate::trace_state::join(&span.span_context.trace_state),
            parent_span_id: decode_id(&span.parent_span_id),
            flags: span.span_context.trace_flags.into(),
            name: span.name,
            kind: span.span_kind as i32,
            start_time_unix_nano: datetime_nanos(span.start_time),
            end_time_unix_nano: datetime_nanos(span.end_time),
            attributes: span.attributes.into_iter().map(Into::into).collect(),
            dropped_attributes_count: span.dropped_attributes,
            events: span.events.into_iter().map(Into::into).collect(),
            dropped_events_count: span.dropped_events,
            links: span.links.into_iter().map(Into::into).collect(),
            dropped_links_count: span.dropped_links,
            status: Some(span.status.into()),
        }
    }
}

impl From<wasi::TraceFlags> for u32 {
    fn from(value: wasi::TraceFlags) -> Self {
        if value.contains(wasi::TraceFlags::SAMPLED) {
            Self::from(TraceFlags::SAMPLED.to_u8())
        } else {
            Self::from(TraceFlags::NOT_SAMPLED.to_u8())
        }
    }
}

impl From<wasi::Event> for Event {
    fn from(event: wasi::Event) -> Self {
        Self {
            time_unix_nano: datetime_nanos(event.time),
            name: event.name,
            attributes: event.attributes.into_iter().map(Into::into).collect(),
            dropped_attributes_count: 0,
        }
    }
}

impl From<wasi::Link> for Link {
    fn from(link: wasi::Link) -> Self {
        Self {
            trace_id: decode_id(&link.span_context.trace_id),
            span_id: decode_id(&link.span_context.span_id),
            trace_state: crate::trace_state::join(&link.span_context.trace_state),
            attributes: link.attributes.into_iter().map(Into::into).collect(),
            dropped_attributes_count: 0,
            flags: link.span_context.trace_flags.into(),
        }
    }
}

impl From<wasi::Status> for Status {
    fn from(value: wasi::Status) -> Self {
        match value {
            wasi::Status::Unset => Self::default(),
            wasi::Status::Error(description) => Self {
                code: StatusCode::Error.into(),
                message: description,
            },
            wasi::Status::Ok => Self {
                code: StatusCode::Ok.into(),
                message: String::new(),
            },
        }
    }
}
