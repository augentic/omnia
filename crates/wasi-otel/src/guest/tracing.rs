//! # Tracing
//!
//! The `opentelemetry` tracing API implemented over the `omnia:otel` records:
//! a span becomes a `span-data` record the moment it ends, and [`export`]
//! hands the buffered records to the host.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use opentelemetry::trace::{
    self as otel, SpanBuilder, SpanContext, SpanId, SpanKind, Status, TraceContextExt as _,
    TraceFlags, TraceId, TraceState,
};
use opentelemetry::{Context, InstrumentationScope, KeyValue};

use crate::guest::generated::omnia::otel::tracing as wasi;

/// Ended spans awaiting export, shared between every span and [`export`].
pub type SpanBuffer = Arc<Mutex<Vec<wasi::SpanData>>>;

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
        // A root span opens a new trace; the host grafts it onto the live
        // host span at export.
        let parent_span = parent_cx.span();
        let parent = parent_span.span_context();
        let (trace_id, parent_span_id, trace_state) = if parent.is_valid() {
            (parent.trace_id(), parent.span_id(), parent.trace_state().clone())
        } else {
            (TraceId::from_bytes(random()), SpanId::INVALID, TraceState::default())
        };
        let span_context = SpanContext::new(
            trace_id,
            SpanId::from_bytes(random()),
            TraceFlags::SAMPLED,
            false,
            trace_state,
        );

        Span {
            context: span_context,
            parent_span_id,
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

/// A span in progress; it records until it ends, explicitly or on drop.
#[derive(Debug)]
pub struct Span {
    context: SpanContext,
    parent_span_id: SpanId,
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

        let data = wasi::SpanData {
            span_context: self.context.clone().into(),
            parent_span_id: self.parent_span_id.to_string(),
            span_kind: self.kind.clone().into(),
            name: std::mem::take(&mut self.name).into_owned(),
            start_time: self.start_time.into(),
            end_time: end_time.into(),
            attributes: std::mem::take(&mut self.attributes).into_iter().map(Into::into).collect(),
            events: std::mem::take(&mut self.events).into_iter().map(Into::into).collect(),
            links: std::mem::take(&mut self.links).into_iter().map(Into::into).collect(),
            status: std::mem::take(&mut self.status).into(),
            instrumentation_scope: self.scope.as_ref().into(),
            dropped_attributes: 0,
            dropped_events: 0,
            dropped_links: 0,
        };
        if let Ok(mut spans) = self.spans.lock() {
            spans.push(data);
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

/// Random id bytes; a per-instance counter stands in should the random
/// source fail, so an id is still unique and never the all-zero invalid one.
fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    if getrandom::fill(&mut bytes).is_err() {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let counter = NEXT.fetch_add(1, Ordering::Relaxed).to_be_bytes();
        bytes[N - counter.len()..].copy_from_slice(&counter);
    }
    bytes
}

/// Export all buffered spans to the host.
pub async fn export(buffer: &SpanBuffer) {
    let spans = take(buffer);
    if spans.is_empty() {
        return;
    }

    if let Err(e) = wasi::export(spans).await {
        tracing::error!("failed to export spans: {e}");
    }
}

// A `MutexGuard` binding inside `export` would make its future `!Send`.
fn take(buffer: &SpanBuffer) -> Vec<wasi::SpanData> {
    buffer.lock().map(|mut spans| std::mem::take(&mut *spans)).unwrap_or_default()
}

impl From<SpanContext> for wasi::SpanContext {
    fn from(sc: SpanContext) -> Self {
        // `Display` zero-pads the ids; `LowerHex` would not, and the host
        // hex-decodes them.
        Self {
            trace_id: sc.trace_id().to_string(),
            span_id: sc.span_id().to_string(),
            trace_flags: sc.trace_flags().into(),
            is_remote: sc.is_remote(),
            trace_state: crate::trace_state::parse(&sc.trace_state().header()),
        }
    }
}

impl From<TraceFlags> for wasi::TraceFlags {
    fn from(tf: TraceFlags) -> Self {
        if tf.is_sampled() { Self::SAMPLED } else { Self::empty() }
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

impl From<Status> for wasi::Status {
    fn from(status: Status) -> Self {
        match status {
            Status::Unset => Self::Unset,
            Status::Error { description } => Self::Error(description.to_string()),
            Status::Ok => Self::Ok,
        }
    }
}
