//! # Subscriber
//!
//! The host's `tracing` subscriber: an initializer for console logging
//! (`EnvFilter` + `fmt` to stderr) with a layer seam telemetry exporters
//! attach through when a deployment wants them ([`SubscriberBuilder::layer`]).
//! Nothing here knows about spans, metrics, or export — that is telemetry,
//! and it lives in `omnia-otlp`, which builds on this seam. The subscriber is
//! the root of the host's whole observability stack, telemetry included, but
//! on its own it is console logging.
//!
//! The subscriber is process-global: the first [`SubscriberBuilder::build`]
//! installs it, and later builds in the same process are no-ops that reuse
//! the first initialization.

use std::sync::{Mutex, PoisonError};

use anyhow::Result;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

// Whether omnia's subscriber is installed. Held for the whole of `build`, so
// two racers cannot both pass the check and race on `try_init`.
static INSTALLED: Mutex<bool> = Mutex::new(false);

/// Outcome of [`SubscriberBuilder::build`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    /// This call installed omnia's subscriber.
    Now,
    /// A subscriber was already set (omnia's own earlier build or an
    /// embedder's); nothing was installed.
    Already,
}

/// Builder for the host's `tracing` subscriber: console logging, plus any
/// telemetry layers attached through [`layer`](Self::layer).
pub struct SubscriberBuilder {
    /// Layers installed beneath the console layers (telemetry exporters
    /// attach here).
    layers: Vec<Box<dyn Layer<Registry> + Send + Sync>>,

    /// Explicit filter directives for the console; unset defers to
    /// `RUST_LOG`.
    filter: Option<String>,

    /// The level the console falls back to when `RUST_LOG` is unset and no
    /// explicit directives are given.
    fallback: LevelFilter,
}

impl Default for SubscriberBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriberBuilder {
    /// Create a builder for a console-only subscriber at the `WARN` fallback.
    #[must_use]
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            filter: None,
            fallback: LevelFilter::WARN,
        }
    }

    /// Adds a layer beneath the console layers; this is where a telemetry
    /// exporter (a span or metrics layer) attaches.
    #[must_use]
    pub fn layer(mut self, layer: impl Layer<Registry> + Send + Sync + 'static) -> Self {
        self.layers.push(Box::new(layer));
        self
    }

    /// Filters the subscriber by `directives` instead of the environment.
    ///
    /// `directives` is a `RUST_LOG` string (`info`, `omnia_core=debug`).
    /// The always-on noisy-dependency mutes still apply.
    #[must_use]
    pub fn filter(mut self, directives: impl Into<String>) -> Self {
        self.filter = Some(directives.into());
        self
    }

    /// Sets the level the subscriber falls back to when the environment sets
    /// no `RUST_LOG`.
    ///
    /// `WARN` when not called. Explicit [`filter`](Self::filter) directives
    /// take precedence over both.
    #[must_use]
    pub const fn fallback(mut self, level: LevelFilter) -> Self {
        self.fallback = level;
        self
    }

    /// Installs the subscriber as the process's global `tracing` subscriber.
    ///
    /// The first call in the process installs it and returns
    /// [`Installed::Now`]; later calls are no-ops that reuse it (this
    /// builder's configuration is ignored) and return [`Installed::Already`],
    /// so embedders and the runtime can each initialize without coordinating.
    ///
    /// # Errors
    ///
    /// Returns an error if the filter directives do not parse.
    pub fn build(self) -> Result<Installed> {
        let mut installed = INSTALLED.lock().unwrap_or_else(PoisonError::into_inner);
        if *installed {
            return Ok(Installed::Already);
        }

        let filter_layer = filter(self.filter.as_deref(), self.fallback, rust_log().as_deref())?;
        // Console tracing goes to stderr: stdout belongs to the guest's
        // semantic output (command mode pipes and JSON envelopes must stay
        // clean of log lines).
        let fmt_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

        // Attached layers sit innermost; the global `EnvFilter` gates the
        // whole stack wherever it sits. An already-set subscriber (an
        // embedder's own tracing setup) is tolerated: their subscriber stays
        // and the runtime keeps running.
        if let Err(error) = Registry::default()
            .with(attached(self.layers))
            .with(filter_layer)
            .with(fmt_layer)
            .try_init()
        {
            tracing::warn!(%error, "a tracing subscriber is already set; omnia's skipped");
            return Ok(Installed::Already);
        }
        *installed = true;
        drop(installed);
        Ok(Installed::Now)
    }
}

// The attached layers as one `Layer`, or `None` when there are none. An empty
// `Vec` is not a no-op layer: it registers every callsite as `Interest::never`
// and hints `LevelFilter::OFF`, which as the innermost layer silences the whole
// stack. `Option::None` is tracing-subscriber's no-op, and `Layered` overrides
// its hint.
fn attached(
    layers: Vec<Box<dyn Layer<Registry> + Send + Sync>>,
) -> Option<Vec<Box<dyn Layer<Registry> + Send + Sync>>> {
    (!layers.is_empty()).then_some(layers)
}

// The process's `RUST_LOG`, read once per build so `filter` stays pure over
// its inputs.
fn rust_log() -> Option<String> {
    std::env::var("RUST_LOG").ok()
}

// The subscriber's filter: explicit `directives` when given, else the
// `rust_log` directives with `fallback` as the level an unset variable falls
// back to, so host warnings (a mount that failed to preopen) reach the
// console without a hand-written filter. Every filter carries the
// noisy-dependency mutes so the output stays readable without a hand-written
// suffix.
fn filter(
    directives: Option<&str>, fallback: LevelFilter, rust_log: Option<&str>,
) -> Result<EnvFilter> {
    let base = match directives {
        Some(directives) => EnvFilter::builder().parse(directives)?,
        None => EnvFilter::builder()
            .with_default_directive(fallback.into())
            .parse_lossy(rust_log.unwrap_or_default()),
    };
    Ok(base
        .add_directive("hyper=off".parse()?)
        .add_directive("h2=off".parse()?)
        .add_directive("tonic=off".parse()?)
        .add_directive("opentelemetry=off".parse()?)
        .add_directive("opentelemetry_sdk=off".parse()?)
        .add_directive("omnia_wasi_otel=off".parse()?))
}

// Unit tests by design: these pin the filter-directive contract, not
// guest–host boundary behavior. Pure over their inputs: the process
// `RUST_LOG` is handed in, never read (other tests run in parallel).
#[cfg(test)]
mod tests {
    use super::{LevelFilter, filter};

    const MUTES: [&str; 6] = [
        "hyper=off",
        "h2=off",
        "tonic=off",
        "opentelemetry=off",
        "opentelemetry_sdk=off",
        "omnia_wasi_otel=off",
    ];

    fn rendered(directives: Option<&str>, fallback: LevelFilter, rust_log: Option<&str>) -> String {
        filter(directives, fallback, rust_log).expect("filter directives parse").to_string()
    }

    fn has(rendered: &str, directive: &str) -> bool {
        rendered.split(',').any(|candidate| candidate == directive)
    }

    #[test]
    fn explicit_directives() {
        let rendered = rendered(Some("info,omnia_core=debug"), LevelFilter::WARN, Some("trace"));
        for directive in ["info", "omnia_core=debug"].into_iter().chain(MUTES) {
            assert!(has(&rendered, directive), "missing `{directive}` in `{rendered}`");
        }
        assert!(!has(&rendered, "trace"), "explicit directives replace `RUST_LOG`: {rendered}");
    }

    #[test]
    fn explicit_off() {
        let rendered = rendered(Some("off"), LevelFilter::WARN, None);
        assert!(has(&rendered, "off"), "{rendered}");
    }

    #[test]
    fn fallback_level() {
        let rendered = rendered(None, LevelFilter::INFO, None);
        assert!(has(&rendered, "info"), "an unset `RUST_LOG` falls back: {rendered}");
        for directive in MUTES {
            assert!(has(&rendered, directive), "missing `{directive}` in `{rendered}`");
        }
    }

    #[test]
    fn rust_log_over_fallback() {
        let rendered = rendered(None, LevelFilter::INFO, Some("omnia_core=debug"));
        assert!(has(&rendered, "omnia_core=debug"), "{rendered}");
        assert!(!has(&rendered, "info"), "a set `RUST_LOG` displaces the fallback: {rendered}");
    }

    #[test]
    fn unparsable_directives() {
        assert!(
            filter(Some("omnia_core=loud"), LevelFilter::WARN, None).is_err(),
            "an unknown level must not parse"
        );
    }
}
