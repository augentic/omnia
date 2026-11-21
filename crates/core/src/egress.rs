//! Host-name patterns: the rule a `hosts` grant and a model worker's
//! `egress` share.

use std::net::IpAddr;

/// Checks that `entry` is a host pattern: a whole host name, or `*.suffix`
/// for a domain and every host beneath it.
///
/// A pattern is labels of ASCII letters, digits, and `-`, joined by `.`. It
/// is never an IP literal, and carries no scheme, path, or port.
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
    if is_ip_literal(name) {
        return Err("is an IP literal, not a host name");
    }
    if name.contains(':') {
        return Err("carries a port");
    }
    let labelled = !name.is_empty()
        && name.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    if !labelled {
        return Err("is not a host name or `*.suffix`");
    }
    Ok(())
}

/// Whether the host pattern `pattern` matches the host name `host`.
///
/// Names compare without regard to ASCII case. An IP literal matches no
/// pattern, so a host held to patterns is reached by name alone.
#[must_use]
pub fn matches_host(pattern: &str, host: &str) -> bool {
    !is_ip_literal(host)
        && pattern.strip_prefix("*.").map_or_else(
            || host.eq_ignore_ascii_case(pattern),
            |domain| at_or_beneath(host, domain),
        )
}

// whether every host `inner` matches, `outer` matches too
pub fn covers_host(outer: &str, inner: &str) -> bool {
    outer.strip_prefix("*.").map_or_else(
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

// `[::1]` is how a URL spells an IPv6 host
fn is_ip_literal(host: &str) -> bool {
    host.starts_with('[') || host.parse::<IpAddr>().is_ok()
}
