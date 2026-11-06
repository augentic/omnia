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

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
pub fn resolve_lend<'d, 'p, D>(
    directories: &'d [(D, String)], path: &'p str,
) -> Option<(&'d D, &'p str)> {
    omnia_backend::lend(directories.iter().map(|(dir, name)| (dir, name.as_str())), path)
}
