//! # Tracing
//!
//! The `opentelemetry` tracing API implemented over the `omnia:otel` records:
//! a span becomes a `span` record the moment it ends, and [`export`] hands
//! the buffered records to the host, one `scope-spans` per scope.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use opentelemetry::trace::{
    self as otel, SpanBuilder, SpanContext, SpanId, SpanKind, Status, TraceContextExt as _,
    TraceFlags, TraceId, TraceState,
};
use opentelemetry::{Context, InstrumentationScope, KeyValue};

use crate::otel::convert::unix_nanos;
use crate::otel::traces as wasi;

/// Ended spans awaiting export, shared between every span and [`export`].
pub type SpanBuffer = Arc<Mutex<Vec<(Arc<InstrumentationScope>, wasi::Span)>>>;

/// Provides tracers whose spans end into a shared [`SpanBuffer`].
#[derive(Clone, Debug, Default)]
pub struct TracerProvider {
    spans: SpanBuffer,
}

impl TracerProvider {
    /// A provider and the buffer its spans end into.
    pub fn new() -> (Self, SpanBuffer) {
        let provider = Self::default();
        let spans = Arc::clone(&provider.spans);
        (provider, spans)
    }
}

impl otel::TracerProvider for TracerProvider {
    type Tracer = Tracer;

    fn tracer_with_scope(&self, scope: InstrumentationScope) -> Tracer {
        Tracer {
            scope: Arc::new(scope),
            spans: Arc::clone(&self.spans),
        }
    }
}

/// Starts spans for one instrumentation scope.
#[derive(Clone, Debug)]
pub struct Tracer {
    scope: Arc<InstrumentationScope>,
    spans: SpanBuffer,
}

impl otel::Tracer for Tracer {
    type Span = Span;

    fn build_with_context(&self, builder: SpanBuilder, parent_cx: &Context) -> Span {
        // a root span joins the host's span, and opens a trace of its own when
        // the host has none
        let parent_span = parent_cx.span();
        let local = parent_span.span_context();
        let parent = if local.is_valid() { Some(local.clone()) } else { host_span_context() };
        let (trace_id, trace_flags, trace_state) = parent.as_ref().map_or_else(
            || (TraceId::from_bytes(random()), TraceFlags::SAMPLED, TraceState::default()),
            |parent| {
                let flags = parent.trace_flags().with_sampled(true);
                (parent.trace_id(), flags, parent.trace_state().clone())
            },
        );
        let context = SpanContext::new(
            trace_id,
            SpanId::from_bytes(random()),
            trace_flags,
            false,
            trace_state,
        );

        Span {
            context,
            parent,
            kind: builder.span_kind.unwrap_or(SpanKind::Internal),
            name: builder.name,
            start_time: builder.start_time.unwrap_or_else(SystemTime::now),
            attributes: builder.attributes.unwrap_or_default(),
            events: builder.events.unwrap_or_default(),
            links: builder.links.unwrap_or_default(),
            status: Status::Unset,
            scope: Arc::clone(&self.scope),
            spans: Arc::clone(&self.spans),
            ended: false,
        }
    }
}

// a context the host sends that does not parse is no parent, and a trace
// state that does not parse is empty
fn host_span_context() -> Option<SpanContext> {
    let host = wasi::current_span_context()?;
    let trace_state = host.trace_state.and_then(|header| header.parse().ok()).unwrap_or_default();
    Some(SpanContext::new(
        TraceId::from_hex(&host.trace_id).ok()?,
        SpanId::from_hex(&host.span_id).ok()?,
        TraceFlags::new(host.trace_flags),
        false,
        trace_state,
    ))
}

// a counter stands in should the random source fail, so an id is never the all-zero invalid one
fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    if getrandom::fill(&mut bytes).is_err() {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let counter = NEXT.fetch_add(1, Ordering::Relaxed).to_be_bytes();
        bytes[N - counter.len()..].copy_from_slice(&counter);
    }
    bytes
}

/// A span in progress; it records until it ends, explicitly or on drop.
#[derive(Debug)]
pub struct Span {
    context: SpanContext,
    parent: Option<SpanContext>,
    kind: SpanKind,
    name: Cow<'static, str>,
    start_time: SystemTime,
    attributes: Vec<KeyValue>,
    events: Vec<otel::Event>,
    links: Vec<otel::Link>,
    status: Status,
    scope: Arc<InstrumentationScope>,
    spans: SpanBuffer,
    ended: bool,
}

impl Span {
    fn end_at(&mut self, end_time: SystemTime) {
        if self.ended {
            return;
        }
        self.ended = true;

        let span = wasi::Span {
            span_context: self.context.clone().into(),
            parent_span_id: self.parent.as_ref().map(|parent| parent.span_id().to_string()),
            parent_is_remote: self.parent.as_ref().map(SpanContext::is_remote),
            name: std::mem::take(&mut self.name).into_owned(),
            kind: self.kind.clone().into(),
            start_time_unix_nano: unix_nanos(self.start_time),
            end_time_unix_nano: unix_nanos(end_time),
            attributes: std::mem::take(&mut self.attributes).into_iter().map(Into::into).collect(),
            dropped_attributes_count: 0,
            events: std::mem::take(&mut self.events).into_iter().map(Into::into).collect(),
            dropped_events_count: 0,
            links: std::mem::take(&mut self.links).into_iter().map(Into::into).collect(),
            dropped_links_count: 0,
            status: std::mem::take(&mut self.status).into(),
        };
        if let Ok(mut spans) = self.spans.lock() {
            spans.push((Arc::clone(&self.scope), span));
        }
    }
}

impl otel::Span for Span {
    fn add_event_with_timestamp<T>(
        &mut self, name: T, timestamp: SystemTime, attributes: Vec<KeyValue>,
    ) where
        T: Into<Cow<'static, str>>,
    {
        if !self.ended {
            self.events.push(otel::Event::new(name, timestamp, attributes, 0));
        }
    }

    fn span_context(&self) -> &SpanContext {
        &self.context
    }

    fn is_recording(&self) -> bool {
        !self.ended
    }

    fn set_attribute(&mut self, attribute: KeyValue) {
        if !self.ended {
            self.attributes.push(attribute);
        }
    }

    // `Ok > Error > Unset`: a status only ever moves up that order, as in the
    // SDK, so `Ok` is final and `Unset` never clears an error.
    fn set_status(&mut self, status: Status) {
        if !self.ended && status > self.status {
            self.status = status;
        }
    }

    fn update_name<T>(&mut self, new_name: T)
    where
        T: Into<Cow<'static, str>>,
    {
        if !self.ended {
            self.name = new_name.into();
        }
    }

    fn add_link(&mut self, span_context: SpanContext, attributes: Vec<KeyValue>) {
        if !self.ended {
            self.links.push(otel::Link::new(span_context, attributes, 0));
        }
    }

    fn end_with_timestamp(&mut self, timestamp: SystemTime) {
        self.end_at(timestamp);
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        self.end_at(SystemTime::now());
    }
}

/// Export all buffered spans to the host.
pub async fn export(buffer: &SpanBuffer) {
    let spans = take(buffer);
    if spans.is_empty() {
        return;
    }

    if let Err(e) = wasi::export(group(spans)).await {
        tracing::error!("failed to export spans: {e}");
    }
}

// A `MutexGuard` binding inside `export` would make its future `!Send`.
fn take(buffer: &SpanBuffer) -> Vec<(Arc<InstrumentationScope>, wasi::Span)> {
    buffer.lock().map_or_default(|mut spans| std::mem::take(&mut *spans))
}

// one group per scope, in the order the scopes first ended a span; a scan
// suits the handful of scopes one export carries
fn group(spans: Vec<(Arc<InstrumentationScope>, wasi::Span)>) -> Vec<wasi::ScopeSpans> {
    let mut groups: Vec<(Arc<InstrumentationScope>, Vec<wasi::Span>)> = Vec::new();
    for (scope, span) in spans {
        match groups.iter_mut().find(|(seen, _)| *seen == scope) {
            Some((_, spans)) => spans.push(span),
            None => groups.push((scope, vec![span])),
        }
    }

    groups
        .into_iter()
        .map(|(scope, spans)| wasi::ScopeSpans {
            scope: Some(scope.as_ref().into()),
            spans,
            schema_url: scope.schema_url().map(Into::into),
        })
        .collect()
}

impl From<SpanContext> for wasi::SpanContext {
    fn from(sc: SpanContext) -> Self {
        // `Display` zero-pads the ids to the length the host requires;
        // `LowerHex` would not
        let trace_state = sc.trace_state().header();
        Self {
            trace_id: sc.trace_id().to_string(),
            span_id: sc.span_id().to_string(),
            trace_flags: sc.trace_flags().to_u8(),
            trace_state: (!trace_state.is_empty()).then_some(trace_state),
        }
    }
}

impl From<SpanKind> for wasi::SpanKind {
    fn from(sk: SpanKind) -> Self {
        match sk {
            SpanKind::Client => Self::Client,
            SpanKind::Server => Self::Server,
            SpanKind::Producer => Self::Producer,
            SpanKind::Consumer => Self::Consumer,
            SpanKind::Internal => Self::Internal,
        }
    }
}

impl From<otel::Event> for wasi::Event {
    fn from(event: otel::Event) -> Self {
        Self {
            time_unix_nano: unix_nanos(event.timestamp),
            name: event.name.into_owned(),
            attributes: event.attributes.into_iter().map(Into::into).collect(),
            dropped_attributes_count: event.dropped_attributes_count,
        }
    }
}

impl From<otel::Link> for wasi::Link {
    fn from(link: otel::Link) -> Self {
        let is_remote = Some(link.span_context.is_remote());
        Self {
            span_context: link.span_context.into(),
            is_remote,
            attributes: link.attributes.into_iter().map(Into::into).collect(),
            dropped_attributes_count: link.dropped_attributes_count,
        }
    }
}

impl From<Status> for wasi::Status {
    fn from(status: Status) -> Self {
        match status {
            Status::Unset => Self::Unset,
            Status::Error { description } => {
                Self::Error((!description.is_empty()).then(|| description.into_owned()))
            }
            Status::Ok => Self::Ok,
        }
    }
}
