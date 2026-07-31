//! W3C `tracestate` header helpers shared by the guest and host bindings.

/// Join key/value pairs into a `tracestate` header.
#[cfg(not(target_arch = "wasm32"))]
pub fn join(pairs: &[(String, String)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")
}

/// Parse a `tracestate` header into key/value pairs, skipping malformed
/// entries.
pub fn parse(header: &str) -> Vec<(String, String)> {
    header
        .split(',')
        .filter_map(|entry| entry.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}
