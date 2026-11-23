//! Name patterns: a whole name, or a prefix that ends in `*`.

/// Whether `name` matches `pattern`, a whole name or a prefix that ends in `*`.
///
/// `*` alone matches every name, and matching is case-sensitive.
#[must_use]
pub fn matches(pattern: &str, name: &str) -> bool {
    pattern.strip_suffix('*').map_or(pattern == name, |prefix| name.starts_with(prefix))
}

// the pattern matching exactly the names both match, when any name does
pub fn narrower<'a>(a: &'a str, b: &'a str) -> Option<&'a str> {
    if covers(b, a) {
        Some(a)
    } else if covers(a, b) {
        Some(b)
    } else {
        None
    }
}

fn covers(outer: &str, inner: &str) -> bool {
    outer.strip_suffix('*').map_or(outer == inner, |prefix| {
        inner.strip_suffix('*').unwrap_or(inner).starts_with(prefix)
    })
}

// Unit tests by design: the matcher is a pure function every list shares.
#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn whole_name() {
        assert!(matches("HOME", "HOME"));
        assert!(!matches("HOME", "HOMEPATH"));
        assert!(!matches("HOME", "home"));
    }

    #[test]
    fn prefix() {
        assert!(matches("ORDERS_*", "ORDERS_URL"));
        assert!(matches("ORDERS_*", "ORDERS_"));
        assert!(!matches("ORDERS_*", "ORDERS"));
        assert!(matches("*", "PATH"));
        assert!(matches("*", ""));
    }
}
