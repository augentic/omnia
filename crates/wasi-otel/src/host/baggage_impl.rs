//! Host side of `omnia:otel/baggage`: the chain's baggage, read and extended.

use std::collections::BTreeMap;

use opentelemetry::KeyValue;
use opentelemetry::baggage::Baggage;

use crate::host::WasiOtelCtxView;
use crate::host::generated::omnia::otel::baggage as wasi;

impl wasi::Host for WasiOtelCtxView<'_> {
    fn baggage(&mut self) -> wasmtime::Result<Vec<(String, String)>> {
        Ok(omnia_core::baggage()
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect())
    }

    fn set_baggage(&mut self, entries: Vec<(String, String)>) -> wasmtime::Result<()> {
        omnia_core::set_baggage(merge(&omnia_core::baggage(), entries));
        Ok(())
    }
}

/// `current` with `entries` applied, each replacing the value under its name.
///
/// The SDK's `Baggage` enforces the W3C grammar and limits on insert, so an
/// entry it refuses is dropped with a warning rather than handed on.
fn merge(
    current: &BTreeMap<String, String>, entries: Vec<(String, String)>,
) -> BTreeMap<String, String> {
    let mut merged: Baggage =
        current.iter().map(|(name, value)| KeyValue::new(name.clone(), value.clone())).collect();
    for (name, value) in entries {
        merged.insert(name.clone(), value.clone());
        if merged.get(&name).is_none_or(|kept| kept.as_str() != value) {
            tracing::warn!(name, "baggage entry dropped: not a token name, or over the W3C limits");
        }
    }
    merged
        .iter()
        .map(|(name, (value, _))| (name.as_str().to_owned(), value.as_str().to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
    }

    #[test]
    fn replaces_by_name() {
        let merged = merge(
            &entries(&[("tenant", "acme"), ("k", "old")]),
            vec![("k".to_owned(), "new".to_owned())],
        );
        assert_eq!(merged, entries(&[("k", "new"), ("tenant", "acme")]));
    }

    // A value may hold the header delimiters a W3C codec would have to encode;
    // nothing here is ever encoded.
    #[test]
    fn delimiters_in_value() {
        let merged = merge(&BTreeMap::new(), vec![("k".to_owned(), "a, b; c=d".to_owned())]);
        assert_eq!(merged, entries(&[("k", "a, b; c=d")]));
    }

    #[test]
    fn non_token_name() {
        let merged = merge(&entries(&[("ok", "v")]), vec![("a b".to_owned(), "v".to_owned())]);
        assert_eq!(merged, entries(&[("ok", "v")]));
    }
}
