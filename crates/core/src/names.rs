//! Name patterns: a whole name, or a prefix that ends in `*`.

/// Whether `name` matches `pattern`, a whole name or a prefix that ends in `*`.
///
/// `*` alone matches every name, and matching is case-sensitive.
#[must_use]
pub fn matches(pattern: &str, name: &str) -> bool {
    pattern.strip_suffix('*').map_or(pattern == name, |prefix| name.starts_with(prefix))
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
