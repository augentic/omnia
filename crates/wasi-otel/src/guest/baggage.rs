//! Baggage: the entries the dispatch chain carries, read and extended, and
//! the one entry this crate reads for itself — the tracing level a guest
//! opens at.

use opentelemetry::KeyValue;
pub use opentelemetry::baggage::Baggage;
use tracing::level_filters::LevelFilter;

use crate::guest::generated::omnia::otel::baggage as wasi;

/// The baggage entry naming the tracing level the chain's guests open at.
///
/// A guest that has chosen its level — a command that has parsed its
/// verbosity flags, say — names it for the guests it dispatches with
/// [`set_baggage`], beside the [`set_filter`](crate::set_filter) reload of
/// its own subscriber:
///
/// ```rust,ignore
/// omnia_wasi_otel::set_filter("info")?;
/// omnia_wasi_otel::set_baggage([(omnia_wasi_otel::LEVEL, "info")]);
/// ```
///
/// Every guest dispatched beneath it then opens its subscriber at that level
/// as its telemetry initializes — before its outermost `#[instrument]` span,
/// so that span is admitted at the level the caller asked for with no reload
/// of the callee's own. `RUST_LOG` applies on top, as it does to
/// [`set_filter`](crate::set_filter). A level crosses, never directives: a
/// callee's per-crate filter is its own to compose over [`level`].
pub const LEVEL: &str = "tracing.level";

/// The baggage the chain currently carries: what this guest was dispatched
/// with, plus what it has set since.
#[must_use]
pub fn baggage() -> Baggage {
    wasi::baggage().into_iter().map(|(name, value)| KeyValue::new(name, value)).collect()
}

/// Sets baggage `entries` for the guests dispatched beneath this one from now
/// on; an entry replaces the value under its name and leaves other names in
/// place.
pub fn set_baggage<K: Into<String>, V: Into<String>>(entries: impl IntoIterator<Item = (K, V)>) {
    let entries: Vec<(String, String)> =
        entries.into_iter().map(|(name, value)| (name.into(), value.into())).collect();
    wasi::set_baggage(&entries);
}

/// The tracing level the chain carries under [`LEVEL`].
///
/// `ERROR` — the level a guest opens at by default — when the chain names
/// none, as at a root that has set none, or when the entry is not a level,
/// which is reported to stderr and ignored. This is the level the guest's
/// subscriber opened at, unless `RUST_LOG` or a
/// [`set_filter`](crate::set_filter) call has said otherwise.
#[must_use]
pub fn level() -> LevelFilter {
    let Some((_, value)) = wasi::baggage().into_iter().find(|(name, _)| name == LEVEL) else {
        return LevelFilter::ERROR;
    };
    value.parse().unwrap_or_else(|err| {
        eprintln!("ignoring `{LEVEL}={value}`: {err}");
        LevelFilter::ERROR
    })
}
