//! Lend path matching and relative subpath validation.

use std::path::{Component, Path};

use anyhow::{Result, ensure};

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

#[cfg(test)]
mod tests {
    use super::check_subpath;

    #[test]
    fn plain_subpath() {
        check_subpath("docs").unwrap();
        check_subpath("docs/guides").unwrap();
        check_subpath(".emery/vcs/repos/abc").unwrap();
    }

    #[test]
    fn complex_subpath() {
        for subpath in ["/abs", "a\\b", "a//b", ".", "..", "a/../b", "./a", "a/"] {
            check_subpath(subpath).unwrap_err();
        }
    }

    #[cfg(windows)]
    #[test]
    fn drive_prefixed_subpath() {
        for subpath in ["C:evil", "C:/evil"] {
            check_subpath(subpath).unwrap_err();
        }
    }
}
