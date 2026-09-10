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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn join_then_parse() {
        let original = pairs(&[("vendor", "opaque"), ("other", "a:b=c")]);
        assert_eq!(parse(&join(&original)), original);
    }

    #[test]
    fn parse_skips_malformed_entries() {
        assert_eq!(
            parse("vendor=opaque,noequals,other=v"),
            pairs(&[("vendor", "opaque"), ("other", "v")])
        );
    }

    #[test]
    fn parse_empty_header() {
        assert!(parse("").is_empty());
    }
}
