//! Preopen mounts, lend path matching, and the mount registry.
//!
//! A deployment may mount node-local directories into a guest's WASI sandbox via
//! `[[mount]]` so a guest can read or edit a mounted directory tree. Two
//! host-side artefacts back that capability:
//!
//! - [`ResolvedPreopen`] — a mount resolved to an absolute host path plus
//!   writability, applied per store when building the WASI context.
//! - [`MountRegistry`] — the host-side source of truth that maps a lent
//!   `wasi:filesystem` descriptor or a lend path back to its mount. It is
//!   built once at startup (opening each directory and capturing its identity),
//!   then shared read-only across every store. The registry — never the
//!   descriptor — supplies the resolved handle: a cap-std `Dir` for bounded
//!   operations and the absolute host path.

#[cfg(not(target_arch = "wasm32"))]
mod registry;

use std::path::{Component, Path};

use anyhow::{Result, ensure};

#[cfg(not(target_arch = "wasm32"))]
pub use self::registry::{Mount, MountRegistry, ResolvedPreopen};

/// Match a deployment-local lend `path` against `(root, name)` preopen pairs.
///
/// Selects the longest preopen name that equals `path` or prefixes it at a `/`
/// boundary, returning the matched root and the remaining subpath (`""` for the
/// mount root itself).
#[must_use]
pub fn lend<'d, 'p, D: 'd>(
    candidates: impl IntoIterator<Item = (&'d D, &'d str)>, path: &'p str,
) -> Option<(&'d D, &'p str)> {
    candidates
        .into_iter()
        .filter_map(|(dir, name)| Some((dir, lend_subpath(name, path)?)))
        .max_by_key(|(_, subpath)| std::cmp::Reverse(subpath.len()))
}

fn lend_subpath<'a>(name: &str, path: &'a str) -> Option<&'a str> {
    if path == name {
        return Some("");
    }
    path.strip_prefix(name)?.strip_prefix('/').filter(|rest| !rest.is_empty())
}

/// Validate that `subpath` is a plain relative path beneath a mount root.
///
/// Rejects leading `/`, `\`, and empty, `.`, `..`, or prefix components.
///
/// # Errors
///
/// Returns an error if `subpath` is not a plain relative path.
pub fn check_subpath(subpath: &str) -> Result<()> {
    let plain = !subpath.starts_with('/')
        && !subpath.contains('\\')
        && subpath.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(subpath).components().all(|part| matches!(part, Component::Normal(_)));
    ensure!(plain, "subpath `{subpath}` is not a plain relative path");
    Ok(())
}
