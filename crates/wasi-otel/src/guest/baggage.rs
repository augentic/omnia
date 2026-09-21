//! Baggage: the entries the dispatch chain carries, read and extended.

use opentelemetry::KeyValue;
pub use opentelemetry::baggage::Baggage;

use crate::guest::generated::omnia::otel::baggage as wasi;

/// The baggage the chain currently carries: what this guest was dispatched
/// with, plus what it has set since.
#[must_use]
pub fn baggage() -> Baggage {
    wasi::baggage().into_iter().map(|(name, value)| KeyValue::new(name, value)).collect()
}

/// Sets baggage `entries` for the guests dispatched beneath this one from now
/// on.
///
/// An entry replaces the value under its name and leaves other names in
/// place; the host drops, with a warning, a name that is not an RFC 7230
/// token or an entry over the W3C limits.
pub fn set_baggage<K: Into<String>, V: Into<String>>(entries: impl IntoIterator<Item = (K, V)>) {
    let entries: Vec<(String, String)> =
        entries.into_iter().map(|(name, value)| (name.into(), value.into())).collect();
    wasi::set_baggage(&entries);
}
