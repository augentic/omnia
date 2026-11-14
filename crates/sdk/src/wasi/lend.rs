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

// The lent root borrows from the preopens and the subpath from the path, so
// a caller may own the one and drop the other.
pub fn resolve_lend<'d, 'p, D>(
    directories: &'d [(D, String)], path: &'p str,
) -> Option<(&'d D, &'p str)> {
    directories
        .iter()
        .filter_map(|(dir, name)| Some((dir, subpath(name, path)?)))
        .max_by_key(|(_, subpath)| std::cmp::Reverse(subpath.len()))
}

fn subpath<'a>(name: &str, path: &'a str) -> Option<&'a str> {
    if path == name {
        return Some("");
    }
    path.strip_prefix(name)?.strip_prefix('/').filter(|rest| !rest.is_empty())
}
