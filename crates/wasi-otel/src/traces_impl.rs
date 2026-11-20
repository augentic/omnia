use omnia_core::HostView;
use opentelemetry::trace::TraceContextExt as _;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::trace::v1::span::{Event, Link, SpanKind};
use opentelemetry_proto::tonic::trace::v1::status::StatusCode;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span, SpanFlags, Status};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use wasmtime::component::{Access, Accessor};

use crate::host::generated::omnia::otel::traces::{self as wasi, Host, HostWithStore};
use crate::host::{Error, OtelClient, Result, WasiOtel, types_impl};

impl<T> HostWithStore<T> for WasiOtel {
    fn current_span_context(_: Access<'_, T, Self>) -> wasmtime::Result<Option<wasi::SpanContext>> {
        let context = tracing::Span::current().context();
        let span = context.span();
        let span_context = span.span_context();
        if !span_context.is_valid() {
            return Ok(None);
        }

        let trace_state = span_context.trace_state().header();
        Ok(Some(wasi::SpanContext {
            trace_id: span_context.trace_id().to_string(),
            span_id: span_context.span_id().to_string(),
            trace_flags: span_context.trace_flags().to_u8(),
            trace_state: (!trace_state.is_empty()).then_some(trace_state),
        }))
    }

    async fn export(
        accessor: &Accessor<T, Self>, scope_spans: Vec<wasi::ScopeSpans>,
    ) -> Result<()> {
        if scope_spans.iter().all(|group| group.spans.is_empty()) {
            return Ok(());
        }
        let scope_spans =
            scope_spans.into_iter().map(ScopeSpans::try_from).collect::<Result<Vec<_>>>()?;
        let Some((resource, schema_url)) = types_impl::resource() else {
            return Ok(());
        };

        let request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(resource),
                scope_spans,
                schema_url,
            }],
        };
        accessor.with(|mut store| store.get().client.export_traces(request)).await?;
        Ok(())
    }
}

impl Host for HostView<'_, dyn OtelClient> {}

impl TryFrom<wasi::ScopeSpans> for ScopeSpans {
    type Error = Error;

    fn try_from(group: wasi::ScopeSpans) -> Result<Self> {
        Ok(Self {
            scope: group.scope.map(TryInto::try_into).transpose()?,
            spans: group.spans.into_iter().map(Span::try_from).collect::<Result<_>>()?,
            schema_url: group.schema_url.unwrap_or_default(),
        })
    }
}

impl TryFrom<wasi::Span> for Span {
    type Error = Error;

    fn try_from(span: wasi::Span) -> Result<Self> {
        let wasi::Span {
            span_context: context,
            parent_span_id,
            parent_is_remote,
            name,
            kind,
            start_time_unix_nano,
            end_time_unix_nano,
            attributes: span_attributes,
            dropped_attributes_count,
            events,
            dropped_events_count,
            links,
            dropped_links_count,
            status,
        } = span;
        let within = |err: Error| err.within(&format!("span `{name}`"));

        let trace_id = types_impl::trace_id(&context.trace_id, "trace id").map_err(within)?;
        let span_id = types_impl::span_id(&context.span_id, "span id").map_err(within)?;
        let parent_span_id = parent_span_id
            .map(|id| types_impl::span_id(&id, "parent span id"))
            .transpose()
            .map_err(within)?;
        let span_attributes = types_impl::attributes(span_attributes).map_err(within)?;
        let events =
            events.into_iter().map(Event::try_from).collect::<Result<Vec<_>>>().map_err(within)?;
        let links =
            links.into_iter().map(Link::try_from).collect::<Result<Vec<_>>>().map_err(within)?;

        Ok(Self {
            trace_id,
            span_id,
            trace_state: context.trace_state.unwrap_or_default(),
            parent_span_id: parent_span_id.unwrap_or_default(),
            flags: flags(context.trace_flags, parent_is_remote),
            name,
            kind: SpanKind::from(kind).into(),
            start_time_unix_nano,
            end_time_unix_nano,
            attributes: span_attributes,
            dropped_attributes_count,
            events,
            dropped_events_count,
            links,
            dropped_links_count,
            status: Some(status.into()),
        })
    }
}

// the W3C trace flags in bits 0 to 7, and whether the parent or linked
// context is remote in bits 8 and 9, as OTLP's `SpanFlags` lays them out
fn flags(trace_flags: u8, is_remote: Option<bool>) -> u32 {
    let has_is_remote = SpanFlags::ContextHasIsRemoteMask as u32;
    let remote = match is_remote {
        None => 0,
        Some(false) => has_is_remote,
        Some(true) => has_is_remote | SpanFlags::ContextIsRemoteMask as u32,
    };
    u32::from(trace_flags) | remote
}

impl From<wasi::SpanKind> for SpanKind {
    fn from(kind: wasi::SpanKind) -> Self {
        match kind {
            wasi::SpanKind::Internal => Self::Internal,
            wasi::SpanKind::Server => Self::Server,
            wasi::SpanKind::Client => Self::Client,
            wasi::SpanKind::Producer => Self::Producer,
            wasi::SpanKind::Consumer => Self::Consumer,
        }
    }
}

impl TryFrom<wasi::Event> for Event {
    type Error = Error;

    fn try_from(event: wasi::Event) -> Result<Self> {
        let event_attributes = types_impl::attributes(event.attributes)
            .map_err(|err| err.within(&format!("event `{}`", event.name)))?;
        Ok(Self {
            time_unix_nano: event.time_unix_nano,
            name: event.name,
            attributes: event_attributes,
            dropped_attributes_count: event.dropped_attributes_count,
        })
    }
}

impl TryFrom<wasi::Link> for Link {
    type Error = Error;

    fn try_from(link: wasi::Link) -> Result<Self> {
        let context = link.span_context;
        Ok(Self {
            trace_id: types_impl::trace_id(&context.trace_id, "link trace id")?,
            span_id: types_impl::span_id(&context.span_id, "link span id")?,
            trace_state: context.trace_state.unwrap_or_default(),
            attributes: types_impl::attributes(link.attributes)
                .map_err(|err| err.within("link"))?,
            dropped_attributes_count: link.dropped_attributes_count,
            flags: flags(context.trace_flags, link.is_remote),
        })
    }
}

impl From<wasi::Status> for Status {
    fn from(status: wasi::Status) -> Self {
        match status {
            wasi::Status::Unset => Self::default(),
            wasi::Status::Ok => Self {
                message: String::new(),
                code: StatusCode::Ok.into(),
            },
            wasi::Status::Error(description) => Self {
                message: description.unwrap_or_default(),
                code: StatusCode::Error.into(),
            },
        }
    }
}
