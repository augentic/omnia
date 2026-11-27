//! Where each signal's telemetry goes, and the transport that carries the
//! host's and the guests' there over one channel per collector.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self as std_mpsc, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use http::Uri;
use http::uri::Scheme;
use opentelemetry_otlp::{
    MetricExporter, OTEL_EXPORTER_OTLP_ENDPOINT, OTEL_EXPORTER_OTLP_HEADERS,
    OTEL_EXPORTER_OTLP_INSECURE, OTEL_EXPORTER_OTLP_METRICS_ENDPOINT,
    OTEL_EXPORTER_OTLP_METRICS_HEADERS, OTEL_EXPORTER_OTLP_METRICS_INSECURE,
    OTEL_EXPORTER_OTLP_METRICS_TIMEOUT, OTEL_EXPORTER_OTLP_TIMEOUT,
    OTEL_EXPORTER_OTLP_TIMEOUT_DEFAULT, OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
    OTEL_EXPORTER_OTLP_TRACES_HEADERS, OTEL_EXPORTER_OTLP_TRACES_INSECURE,
    OTEL_EXPORTER_OTLP_TRACES_TIMEOUT, SpanExporter, WithExportConfig, WithTonicConfig,
};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::metrics::v1::metrics_service_client::MetricsServiceClient;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::trace_service_client::TraceServiceClient;
use prost::Message;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;
use tonic::metadata::{MetadataKey, MetadataMap, MetadataValue};
use tonic::transport::{Channel, Endpoint};

static TRANSPORT: OnceLock<Transport> = OnceLock::new();

/// What became of guest telemetry handed to [`forward_traces`] or
/// [`forward_metrics`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forwarded {
    /// Queued for the signal's collector, behind the exports before it.
    Queued,

    /// Dropped, since the signal has no collector.
    NoCollector,

    /// Dropped, since the signal's queue already holds 16 `MiB`.
    Full,

    /// Dropped, since one export carries at most 4 `MiB`.
    TooLarge,
}

/// Queues a guest's span export for the collector the host's spans go to.
///
/// The export carries the endpoint, headers, and timeout of the host's own
/// spans, and is sent in the background in one attempt, so the answer never
/// waits on the collector. [`flush`](super::flush) waits for what is queued.
#[must_use]
pub fn forward_traces(request: ExportTraceServiceRequest) -> Forwarded {
    TRANSPORT
        .get()
        .and_then(|transport| transport.traces.as_ref())
        .map_or(Forwarded::NoCollector, |route| route.queue.forward(request))
}

/// Queues a guest's metric export for the collector the host's metrics go
/// to.
///
/// The export carries the endpoint, headers, and timeout of the host's own
/// metrics, and is sent in the background in one attempt, so the answer
/// never waits on the collector. [`flush`](super::flush) waits for what is
/// queued.
#[must_use]
pub fn forward_metrics(request: ExportMetricsServiceRequest) -> Forwarded {
    TRANSPORT
        .get()
        .and_then(|transport| transport.metrics.as_ref())
        .map_or(Forwarded::NoCollector, |route| route.queue.forward(request))
}

// each signal's target; `None` where the signal has no endpoint
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Targets {
    pub(super) traces: Option<Target>,
    pub(super) metrics: Option<Target>,
}

impl Targets {
    // `endpoint` is the builder's, which wins over every variable
    pub(super) fn resolve(
        endpoint: Option<&str>, env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        Ok(Self {
            traces: Target::resolve(&TRACES, endpoint, &env)?,
            metrics: Target::resolve(&METRICS, endpoint, &env)?,
        })
    }
}

// Where one signal exports, as `opentelemetry-otlp` resolves it from the same
// builder and environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    endpoint: Uri,
    headers: Vec<(String, String)>,
    timeout: Duration,
}

impl Target {
    // An `https://` endpoint is an error, since the channel carries no TLS. A
    // schemeless one is `https://` unless the signal is insecure.
    fn resolve(
        variables: &Variables, endpoint: Option<&str>, env: &impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>> {
        let set = |name: &'static str| {
            env(name).filter(|value| !value.is_empty()).map(|value| (value, name))
        };
        let Some((endpoint, source)) = endpoint
            .filter(|endpoint| !endpoint.is_empty())
            .map(|endpoint| (endpoint.to_owned(), "`Telemetry::endpoint`"))
            .or_else(|| set(variables.endpoint))
            .or_else(|| set(SHARED.endpoint))
        else {
            return Ok(None);
        };

        // the scheme, from `INSECURE` where the endpoint names none
        let insecure = env(variables.insecure)
            .or_else(|| env(SHARED.insecure))
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        let endpoint = with_scheme(endpoint, insecure);
        let uri: Uri =
            endpoint.parse().with_context(|| format!("{source}: `{endpoint}` is not a URI"))?;
        if uri.scheme() == Some(&Scheme::HTTPS) {
            bail!(
                "{source}: `{endpoint}` needs TLS, which OTLP export does not support; use an \
                 `http://` endpoint"
            );
        }

        // an empty per-signal variable stands, as it does for `opentelemetry-otlp`
        let headers = env(variables.headers)
            .or_else(|| env(SHARED.headers))
            .map_or_default(|value| headers(&value));
        let millis = |name| env(name).and_then(|value| value.parse().ok());
        let timeout = millis(variables.timeout)
            .or_else(|| millis(SHARED.timeout))
            .map_or(OTEL_EXPORTER_OTLP_TIMEOUT_DEFAULT, Duration::from_millis);

        Ok(Some(Self {
            endpoint: uri,
            headers,
            timeout,
        }))
    }
}

// a signal's variables, each falling back to its `SHARED` form
struct Variables {
    endpoint: &'static str,
    headers: &'static str,
    timeout: &'static str,
    insecure: &'static str,
}

const TRACES: Variables = Variables {
    endpoint: OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
    headers: OTEL_EXPORTER_OTLP_TRACES_HEADERS,
    timeout: OTEL_EXPORTER_OTLP_TRACES_TIMEOUT,
    insecure: OTEL_EXPORTER_OTLP_TRACES_INSECURE,
};

const METRICS: Variables = Variables {
    endpoint: OTEL_EXPORTER_OTLP_METRICS_ENDPOINT,
    headers: OTEL_EXPORTER_OTLP_METRICS_HEADERS,
    timeout: OTEL_EXPORTER_OTLP_METRICS_TIMEOUT,
    insecure: OTEL_EXPORTER_OTLP_METRICS_INSECURE,
};

const SHARED: Variables = Variables {
    endpoint: OTEL_EXPORTER_OTLP_ENDPOINT,
    headers: OTEL_EXPORTER_OTLP_HEADERS,
    timeout: OTEL_EXPORTER_OTLP_TIMEOUT,
    insecure: OTEL_EXPORTER_OTLP_INSECURE,
};

// an explicit scheme stands, `unix://` included
fn with_scheme(endpoint: String, insecure: bool) -> String {
    let schemed =
        endpoint.split_once("://").is_some_and(|(scheme, _)| !scheme.contains(['/', '?', '#']));
    if schemed {
        endpoint
    } else if insecure {
        format!("http://{endpoint}")
    } else {
        format!("https://{endpoint}")
    }
}

// `key=value` pairs, comma-separated, each value percent-decoded where it
// decodes; a pair missing its key or value is dropped
fn headers(value: &str) -> Vec<(String, String)> {
    value
        .split_terminator(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
        .map(|(key, value)| {
            (key.to_owned(), percent_decode(value).unwrap_or_else(|| value.to_owned()))
        })
        .collect()
}

// `None` when an escape is malformed or the bytes are not UTF-8
fn percent_decode(value: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut rest = value.as_bytes();
    while let [byte, tail @ ..] = rest {
        if *byte == b'%' {
            let hex = tail.get(..2)?;
            decoded.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            rest = &tail[2..];
        } else {
            decoded.push(*byte);
            rest = tail;
        }
    }
    String::from_utf8(decoded).ok()
}

// The runtime the channels and the guest queues' drains run on. It is its
// own, so export neither needs the caller's runtime nor ends with it.
#[derive(Default)]
pub(super) struct Transport {
    // `None` when no signal has a target
    runtime: Option<Runtime>,
    traces: Option<Route<ExportTraceServiceRequest>>,
    metrics: Option<Route<ExportMetricsServiceRequest>>,
}

impl Transport {
    pub(super) fn start(targets: &Targets) -> Result<Self> {
        if targets.traces.is_none() && targets.metrics.is_none() {
            return Ok(Self::default());
        }
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("omnia-telemetry")
            .enable_all()
            .build()
            .context("starting the telemetry runtime")?;

        // a lazy channel and a drain each spawn onto the runtime they start in
        let context = runtime.enter();
        let mut channels = Channels::default();
        let traces = targets.traces.as_ref().map(|target| Route::start(target, &mut channels));
        let metrics = targets.metrics.as_ref().map(|target| Route::start(target, &mut channels));
        drop(context);

        Ok(Self {
            runtime: Some(runtime),
            traces,
            metrics,
        })
    }

    pub(super) fn span_exporter(&self) -> Result<Option<SpanExporter>> {
        self.traces.as_ref().map(|route| route.link.span_exporter()).transpose()
    }

    pub(super) fn metric_exporter(&self) -> Result<Option<MetricExporter>> {
        self.metrics.as_ref().map(|route| route.link.metric_exporter()).transpose()
    }

    pub(super) fn publish(self) -> Result<()> {
        TRANSPORT.set(self).map_err(|transport| {
            transport.shutdown();
            anyhow!("telemetry transport already published")
        })
    }

    // a runtime dropped inside another's context panics, so it is let go in the background
    pub(super) fn shutdown(self) {
        if let Some(runtime) = self.runtime {
            runtime.shutdown_background();
        }
    }
}

// One lazily connected channel per endpoint and timeout, which the signals
// naming both share.
#[derive(Default)]
struct Channels(Vec<(Uri, Duration, Channel)>);

impl Channels {
    fn open(&mut self, target: &Target) -> Channel {
        let opened = self.0.iter().find(|(endpoint, timeout, _)| {
            *endpoint == target.endpoint && *timeout == target.timeout
        });
        if let Some((.., channel)) = opened {
            return channel.clone();
        }
        let channel = Endpoint::from(target.endpoint.clone())
            .timeout(target.timeout)
            .connect_timeout(target.timeout)
            .connect_lazy();
        self.0.push((target.endpoint.clone(), target.timeout, channel.clone()));
        channel
    }
}

// one signal's way to its collector, and the queue its guest telemetry waits in
struct Route<R> {
    link: Link,
    queue: Queue<R>,
}

impl<R: Export> Route<R> {
    fn start(target: &Target, channels: &mut Channels) -> Self {
        let link = Link {
            channel: channels.open(target),
            metadata: metadata(&target.headers),
            timeout: target.timeout,
        };
        let (queue, items) = Queue::new();
        tokio::spawn(drain(items, link.clone(), Arc::clone(&queue.held)));
        Self { link, queue }
    }

    fn mark(&self) -> Option<(Duration, std_mpsc::Receiver<()>)> {
        self.queue.mark().map(|marker| (self.link.timeout, marker))
    }
}

// the headers a channel can carry; one `tonic` refuses is dropped, as `opentelemetry-otlp` drops it
fn metadata(headers: &[(String, String)]) -> MetadataMap {
    let mut metadata = MetadataMap::new();
    for (key, value) in headers {
        if let (Ok(key), Ok(value)) =
            (MetadataKey::from_bytes(key.as_bytes()), MetadataValue::try_from(value.as_str()))
        {
            metadata.insert(key, value);
        }
    }
    metadata
}

// a signal's channel and what each export over it carries
#[derive(Clone)]
struct Link {
    channel: Channel,
    metadata: MetadataMap,
    timeout: Duration,
}

impl Link {
    fn span_exporter(&self) -> Result<SpanExporter> {
        Ok(SpanExporter::builder()
            .with_tonic()
            .with_channel(self.channel.clone())
            .with_metadata(self.metadata.clone())
            .with_timeout(self.timeout)
            .build()?)
    }

    fn metric_exporter(&self) -> Result<MetricExporter> {
        Ok(MetricExporter::builder()
            .with_tonic()
            .with_channel(self.channel.clone())
            .with_metadata(self.metadata.clone())
            .with_timeout(self.timeout)
            .build()?)
    }

    async fn send<R: Export>(&self, export: R) {
        let mut request = tonic::Request::new(export);
        *request.metadata_mut() = self.metadata.clone();
        request.set_timeout(self.timeout);
        match tokio::time::timeout(self.timeout, R::send(self.channel.clone(), request)).await {
            Ok(Ok(0)) => {}
            Ok(Ok(rejected)) => {
                tracing::debug!(
                    rejected,
                    "telemetry: the collector rejected part of a guest export"
                );
            }
            Ok(Err(status)) => tracing::debug!(%status, "telemetry: a guest export failed"),
            Err(_elapsed) => tracing::debug!("telemetry: a guest export timed out"),
        }
    }
}

// The encoded bytes one signal's guest telemetry may hold, queued or in
// flight. Past it an export is refused rather than held.
const QUEUE_BYTES: usize = 16 << 20;

// The most one export carries: the `omnia:otel` limit for one call, and a
// collector's default gRPC receive limit.
const EXPORT_BYTES: usize = 4 << 20;

struct Queue<R> {
    items: mpsc::UnboundedSender<Item<R>>,
    held: Arc<AtomicUsize>,
}

enum Item<R> {
    Export(R, usize),

    // dropped once every export queued before it is sent or has failed,
    // which ends its receiver's wait
    Marker(std_mpsc::Sender<()>),
}

impl<R: Export> Queue<R> {
    fn new() -> (Self, mpsc::UnboundedReceiver<Item<R>>) {
        let (items, receiver) = mpsc::unbounded_channel();
        let queue = Self {
            items,
            held: Arc::default(),
        };
        (queue, receiver)
    }

    fn forward(&self, export: R) -> Forwarded {
        let bytes = export.encoded_len();
        if bytes > EXPORT_BYTES {
            return Forwarded::TooLarge;
        }
        let reserved = self.held.try_update(Ordering::AcqRel, Ordering::Acquire, |held| {
            held.checked_add(bytes).filter(|&held| held <= QUEUE_BYTES)
        });
        if reserved.is_err() {
            return Forwarded::Full;
        }
        if self.items.send(Item::Export(export, bytes)).is_err() {
            self.held.fetch_sub(bytes, Ordering::AcqRel);
            return Forwarded::Full;
        }
        Forwarded::Queued
    }

    fn mark(&self) -> Option<std_mpsc::Receiver<()>> {
        let (marker, receiver) = std_mpsc::channel();
        self.items.send(Item::Marker(marker)).ok()?;
        Some(receiver)
    }
}

// Sends a signal's guest exports in order, each merged with those waiting
// behind it up to `EXPORT_BYTES`. One attempt each: a failure is logged at
// debug and the export dropped.
async fn drain<R: Export>(
    mut items: mpsc::UnboundedReceiver<Item<R>>, link: Link, held: Arc<AtomicUsize>,
) {
    while let Some(item) = items.recv().await {
        let mut next = Some(item);
        while let Some(item) = next.take() {
            let (mut export, mut bytes) = match item {
                Item::Export(export, bytes) => (export, bytes),
                Item::Marker(marker) => {
                    drop(marker);
                    continue;
                }
            };

            // merge what is already waiting
            while let Ok(item) = items.try_recv() {
                match item {
                    Item::Export(more, size) if bytes + size <= EXPORT_BYTES => {
                        export.append(more);
                        bytes += size;
                    }
                    item => {
                        next = Some(item);
                        break;
                    }
                }
            }

            link.send(export).await;
            held.fetch_sub(bytes, Ordering::AcqRel);
        }
    }
}

// a guest export request, which the drain merges and sends
trait Export: Message + Default + Send + 'static {
    fn append(&mut self, other: Self);

    // answers how many of the request's items the collector rejected
    fn send(
        channel: Channel, request: tonic::Request<Self>,
    ) -> impl Future<Output = Result<i64, tonic::Status>> + Send;
}

impl Export for ExportTraceServiceRequest {
    fn append(&mut self, other: Self) {
        self.resource_spans.extend(other.resource_spans);
    }

    async fn send(channel: Channel, request: tonic::Request<Self>) -> Result<i64, tonic::Status> {
        let response = TraceServiceClient::new(channel).export(request).await?;
        Ok(response.into_inner().partial_success.map_or(0, |partial| partial.rejected_spans))
    }
}

impl Export for ExportMetricsServiceRequest {
    fn append(&mut self, other: Self) {
        self.resource_metrics.extend(other.resource_metrics);
    }

    async fn send(channel: Channel, request: tonic::Request<Self>) -> Result<i64, tonic::Status> {
        let response = MetricsServiceClient::new(channel).export(request).await?;
        Ok(response.into_inner().partial_success.map_or(0, |partial| partial.rejected_data_points))
    }
}

// The markers `flush` sends behind the guest exports queued before it, each
// waited for at most its signal's timeout from when it was sent.
pub(super) struct Flush {
    sent: Instant,
    markers: Vec<(Duration, std_mpsc::Receiver<()>)>,
}

impl Flush {
    pub(super) fn begin() -> Self {
        let markers = TRANSPORT.get().map_or_else(Vec::new, |transport| {
            let traces = transport.traces.as_ref().and_then(Route::mark);
            let metrics = transport.metrics.as_ref().and_then(Route::mark);
            traces.into_iter().chain(metrics).collect()
        });
        Self {
            sent: Instant::now(),
            markers,
        }
    }

    pub(super) fn wait(self) {
        for (timeout, marker) in self.markers {
            let left = timeout.saturating_sub(self.sent.elapsed());
            if marker.recv_timeout(left) == Err(RecvTimeoutError::Timeout) {
                tracing::debug!("telemetry: guest exports outlasted the flush");
            }
        }
    }
}

// the resolution `opentelemetry-otlp` would make from the same environment,
// which only a collector observes end to end, and the queue's budget, which
// no guest fills in a test's time
#[cfg(test)]
mod tests {
    mod resolve {
        use std::time::Duration;

        use http::Uri;

        use super::super::{Target, Targets};

        // the environment is a closure, never the process's
        fn env(set: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
            move |name| {
                set.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
            }
        }

        fn target(endpoint: &'static str) -> Target {
            Target {
                endpoint: Uri::from_static(endpoint),
                headers: Vec::new(),
                timeout: Duration::from_secs(10),
            }
        }

        fn both(target: &Target) -> Targets {
            Targets {
                traces: Some(target.clone()),
                metrics: Some(target.clone()),
            }
        }

        #[test]
        fn nothing_configured() {
            assert_eq!(Targets::resolve(None, env(&[])).unwrap(), Targets::default());
        }

        #[test]
        fn builder_endpoint() {
            let resolved = Targets::resolve(Some("http://builder:4317"), env(&[])).unwrap();
            assert_eq!(resolved, both(&target("http://builder:4317")));
        }

        // the builder wins over every variable, and a signal's own over the shared one
        #[test]
        fn precedence() {
            let set = env(&[
                ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://shared:4317"),
                ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", "http://traces:4317"),
            ]);
            let resolved = Targets::resolve(None, &set).unwrap();
            assert_eq!(
                resolved,
                Targets {
                    traces: Some(target("http://traces:4317")),
                    metrics: Some(target("http://shared:4317")),
                }
            );

            let resolved = Targets::resolve(Some("http://builder:4317"), &set).unwrap();
            assert_eq!(resolved, both(&target("http://builder:4317")));
        }

        #[test]
        fn per_signal_endpoint() {
            let resolved = Targets::resolve(
                None,
                env(&[("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", "http://metrics:4317")]),
            )
            .unwrap();
            assert_eq!(
                resolved,
                Targets {
                    traces: None,
                    metrics: Some(target("http://metrics:4317")),
                }
            );
        }

        // an `=` left in a profile names no endpoint
        #[test]
        fn empty_endpoints() {
            let empty = |_: &str| Some(String::new());
            assert_eq!(Targets::resolve(Some(""), empty).unwrap(), Targets::default());

            let resolved = Targets::resolve(
                Some(""),
                env(&[
                    ("OTEL_EXPORTER_OTLP_ENDPOINT", ""),
                    ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", "http://traces:4317"),
                ]),
            )
            .unwrap();
            assert_eq!(
                resolved,
                Targets {
                    traces: Some(target("http://traces:4317")),
                    metrics: None,
                }
            );
        }

        // a value percent-decodes where it can, and a pair missing a part is dropped
        #[test]
        fn headers() {
            let resolved = Targets::resolve(
                None,
                env(&[
                    ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://shared:4317"),
                    (
                        "OTEL_EXPORTER_OTLP_HEADERS",
                        "api-key=secret, tenant = a%20b,raw=%zz,=orphan,empty=,junk,",
                    ),
                ]),
            )
            .unwrap();
            let headers = vec![
                ("api-key".to_owned(), "secret".to_owned()),
                ("tenant".to_owned(), "a b".to_owned()),
                ("raw".to_owned(), "%zz".to_owned()),
            ];
            assert_eq!(resolved.traces.unwrap().headers, headers);
            assert_eq!(resolved.metrics.unwrap().headers, headers);
        }

        // a signal's own variable stands even when empty, as it does for `opentelemetry-otlp`
        #[test]
        fn per_signal_headers() {
            let resolved = Targets::resolve(
                None,
                env(&[
                    ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://shared:4317"),
                    ("OTEL_EXPORTER_OTLP_HEADERS", "tenant=shared"),
                    ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", ""),
                    ("OTEL_EXPORTER_OTLP_METRICS_HEADERS", "tenant=metrics"),
                ]),
            )
            .unwrap();
            assert_eq!(resolved.traces.unwrap().headers, Vec::new());
            assert_eq!(
                resolved.metrics.unwrap().headers,
                vec![("tenant".to_owned(), "metrics".to_owned())]
            );
        }

        // a value that is not whole milliseconds falls through to the next
        #[test]
        fn timeouts() {
            let resolved = Targets::resolve(
                None,
                env(&[
                    ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://shared:4317"),
                    ("OTEL_EXPORTER_OTLP_TIMEOUT", "2500"),
                    ("OTEL_EXPORTER_OTLP_TRACES_TIMEOUT", "soon"),
                    ("OTEL_EXPORTER_OTLP_METRICS_TIMEOUT", "500"),
                ]),
            )
            .unwrap();
            assert_eq!(resolved.traces.unwrap().timeout, Duration::from_millis(2500));
            assert_eq!(resolved.metrics.unwrap().timeout, Duration::from_millis(500));

            let resolved = Targets::resolve(
                Some("http://builder:4317"),
                env(&[("OTEL_EXPORTER_OTLP_TIMEOUT", "1.5")]),
            )
            .unwrap();
            assert_eq!(resolved.traces.unwrap().timeout, Duration::from_secs(10));
        }

        #[test]
        fn schemeless_endpoint() {
            let resolved = Targets::resolve(
                Some("collector:4317"),
                env(&[("OTEL_EXPORTER_OTLP_INSECURE", "TRUE")]),
            )
            .unwrap();
            assert_eq!(resolved, both(&target("http://collector:4317")));
        }

        // the channel carries no TLS, which a schemeless endpoint gets unless insecure
        #[test]
        fn tls_endpoint() {
            let error = Targets::resolve(
                None,
                env(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "https://collector:4317")]),
            )
            .unwrap_err();
            assert!(error.to_string().contains("OTEL_EXPORTER_OTLP_ENDPOINT"), "{error}");

            let error = Targets::resolve(
                Some("collector:4317"),
                env(&[("OTEL_EXPORTER_OTLP_TRACES_INSECURE", "true")]),
            )
            .unwrap_err();
            assert!(error.to_string().contains("https://collector:4317"), "{error}");
        }
    }

    mod queue {
        use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
        use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
        use prost::Message;

        use super::super::{EXPORT_BYTES, Forwarded, QUEUE_BYTES, Queue};

        fn spans(name: usize) -> ExportTraceServiceRequest {
            let span = Span {
                name: "x".repeat(name),
                ..Span::default()
            };
            ExportTraceServiceRequest {
                resource_spans: vec![ResourceSpans {
                    scope_spans: vec![ScopeSpans {
                        spans: vec![span],
                        ..ScopeSpans::default()
                    }],
                    ..ResourceSpans::default()
                }],
            }
        }

        // nothing drains, so every queued byte stays held
        #[test]
        fn budget() {
            let (queue, _items) = Queue::new();
            let export = spans(EXPORT_BYTES / 4);
            for _ in 0..QUEUE_BYTES / export.encoded_len() {
                assert_eq!(queue.forward(export.clone()), Forwarded::Queued);
            }
            assert_eq!(queue.forward(export), Forwarded::Full);
            assert_eq!(queue.forward(spans(EXPORT_BYTES)), Forwarded::TooLarge);
        }
    }
}
