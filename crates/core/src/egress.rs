//! Host-name patterns: the rule a `hosts` grant and a model worker's
//! `egress` share.

use std::net::IpAddr;

/// Checks that `entry` is a host pattern: a whole host name, or `*.suffix`
/// for a domain and every host beneath it.
///
/// A pattern is labels of ASCII letters, digits, and `-`, joined by `.`. It
/// carries no scheme, path, or port, and is never an IP literal. Its last
/// label is not a number, decimal or `0x` hex, since a URL parser reads a
/// name that ends in one as an IP address.
///
/// # Errors
///
/// Returns why `entry` is not a host pattern, as a phrase that follows it,
/// such as `carries a port`.
pub fn host_pattern(entry: &str) -> Result<(), &'static str> {
    if entry.contains("://") {
        return Err("carries a scheme");
    }
    if entry.contains('/') {
        return Err("carries a path");
    }
    let name = entry.trim_prefix("*.");
    if name.starts_with('[') || name.parse::<IpAddr>().is_ok() {
        return Err("is an IP literal, not a host name");
    }
    if name.contains(':') {
        return Err("carries a port");
    }
    if !is_labelled(name) {
        return Err("is not a host name or `*.suffix`");
    }
    if ends_in_number(name) {
        return Err("ends in a numeric label, which reads as an IP address");
    }
    Ok(())
}

// A host matches as a plain name alone. A URL parser reads an IP literal or
// a numeric last label as an address, and drops a trailing dot before it
// looks, so none of those matches any pattern, `*` included.
pub fn matches_host(pattern: &str, host: &str) -> bool {
    if !is_labelled(host) || ends_in_number(host) {
        return false;
    }
    pattern == "*"
        || pattern.strip_prefix("*.").map_or_else(
            || host.eq_ignore_ascii_case(pattern),
            |domain| at_or_beneath(host, domain),
        )
}

// whether every host `inner` matches, `outer` matches too
pub fn covers_host(outer: &str, inner: &str) -> bool {
    outer == "*"
        || outer.strip_prefix("*.").map_or_else(
            || !inner.starts_with("*.") && outer.eq_ignore_ascii_case(inner),
            |domain| at_or_beneath(inner.trim_prefix("*."), domain),
        )
}

// domains nest, so two patterns meet in the narrower of them, or not at all
pub fn narrower_host<'a>(a: &'a str, b: &'a str) -> Option<&'a str> {
    if covers_host(b, a) {
        Some(a)
    } else if covers_host(a, b) {
        Some(b)
    } else {
        None
    }
}

fn at_or_beneath(host: &str, domain: &str) -> bool {
    host.eq_ignore_ascii_case(domain)
        || host.len().checked_sub(domain.len() + 1).is_some_and(|dot| {
            host.as_bytes().get(dot) == Some(&b'.')
                && host.get(dot + 1..).is_some_and(|tail| tail.eq_ignore_ascii_case(domain))
        })
}

fn is_labelled(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

// the WHATWG URL parser's test, under which `0x` alone is the number zero
fn ends_in_number(name: &str) -> bool {
    let last = name.rsplit_once('.').map_or(name, |(_, last)| last);
    last.strip_prefix("0x").or_else(|| last.strip_prefix("0X")).map_or_else(
        || !last.is_empty() && last.bytes().all(|byte| byte.is_ascii_digit()),
        |hex| hex.bytes().all(|byte| byte.is_ascii_hexdigit()),
    )
}
