//! Resolution of a deployment-local path onto a guest's preopens.
//!
//! A capability that lends a directory to the host names it with a plain
//! path — `"."` for the project mount, `"/mount/sub"` or `"./sub"` beneath
//! one — and [`WasiProvider`](crate::WasiProvider) resolves that path against
//! the guest's preopens at the call site: the longest preopen whose name
//! equals the path or prefixes it at a `/` boundary is the lent root
//! descriptor, and the remainder rides as the grant's subpath (empty for the
//! mount itself). The one rule serves every lending capability, so a path
//! lends the same directory whichever interface carries it.

#[cfg(any(all(target_arch = "wasm32", target_os = "wasi"), test))]
pub fn resolve_lend<'d, 'p, D>(
    directories: &'d [(D, String)], path: &'p str,
) -> Option<(&'d D, &'p str)> {
    omnia_backend::lend(directories.iter().map(|(dir, name)| (dir, name.as_str())), path)
}

#[cfg(test)]
mod tests {
    use super::resolve_lend;

    fn preopens() -> Vec<(u8, String)> {
        vec![(0, ".".to_string()), (1, "/emery-workspaces".to_string())]
    }

    #[test]
    fn resolves_mount() {
        let dirs = preopens();
        assert_eq!(resolve_lend(&dirs, ".").map(|(_, sub)| sub), Some(""));
        assert_eq!(resolve_lend(&dirs, "./nested").map(|(_, sub)| sub), Some("nested"));
        assert_eq!(resolve_lend(&dirs, "/emery-workspaces/ws-1").map(|(_, sub)| sub), Some("ws-1"));
        assert_eq!(
            resolve_lend(&dirs, "/emery-workspaces/ws-1/nested").map(|(_, sub)| sub),
            Some("ws-1/nested")
        );
    }

    #[test]
    fn refuses_paths() {
        let dirs = preopens();
        assert!(resolve_lend(&dirs, "/elsewhere").is_none());
        assert!(resolve_lend(&dirs, "/emery-workspaces-evil/x").is_none());
        assert!(resolve_lend(&dirs, "/emery-workspaces/").is_none());
    }
}
