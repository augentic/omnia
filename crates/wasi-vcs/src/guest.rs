//! # WASI VCS Guest

// Bindings for the `omnia:vcs` world.
mod generated {
    #![expect(missing_docs, clippy::same_length_and_capacity, reason = "wit-bindgen output")]
    wit_bindgen::generate!({
        world: "imports",
        path: "wit",
        with: {
            "wasi:filesystem/types@0.3.2": wasip3::filesystem::types,
            "wasi:clocks/system-clock@0.3.2": wasip3::clocks::system_clock,
            "wasi:clocks/types@0.3.2": wasip3::clocks::types,
        },
    });
}

pub use self::generated::omnia::vcs::*;

impl From<omnia_backend::vcs::Rule> for store::Rule {
    fn from(rule: omnia_backend::vcs::Rule) -> Self {
        Self {
            paths: rule.paths,
            strategy: rule.strategy.into(),
        }
    }
}

impl From<omnia_backend::vcs::Strategy> for store::Strategy {
    fn from(strategy: omnia_backend::vcs::Strategy) -> Self {
        match strategy {
            omnia_backend::vcs::Strategy::Union => Self::Union,
            omnia_backend::vcs::Strategy::Ours => Self::Ours,
            omnia_backend::vcs::Strategy::Theirs => Self::Theirs,
        }
    }
}

impl From<store::Merged> for omnia_backend::vcs::Merged {
    fn from(merged: store::Merged) -> Self {
        Self {
            commit: merged.commit,
            conflicts: merged.conflicts,
        }
    }
}

impl From<store::Entry> for omnia_backend::vcs::Entry {
    fn from(entry: store::Entry) -> Self {
        Self {
            id: entry.id,
            message: entry.message,
        }
    }
}

impl From<workspace::Change> for omnia_backend::vcs::Change {
    fn from(change: workspace::Change) -> Self {
        Self {
            path: change.path,
            kind: change.kind.into(),
        }
    }
}

impl From<workspace::ChangeKind> for omnia_backend::vcs::ChangeKind {
    fn from(kind: workspace::ChangeKind) -> Self {
        match kind {
            workspace::ChangeKind::Added => Self::Added,
            workspace::ChangeKind::Modified => Self::Modified,
            workspace::ChangeKind::Deleted => Self::Deleted,
        }
    }
}

impl From<omnia_backend::vcs::CloneOptions> for transport::CloneOptions {
    fn from(options: omnia_backend::vcs::CloneOptions) -> Self {
        Self { depth: options.depth }
    }
}

impl From<types::Error> for omnia_backend::vcs::Error {
    fn from(error: types::Error) -> Self {
        match error {
            types::Error::NotARepository => Self::NotARepository,
            types::Error::Exists(detail) => Self::Exists(detail),
            types::Error::NotFound(detail) => Self::NotFound(detail),
            types::Error::Pending(paths) => Self::Pending(paths),
            types::Error::Access(detail) => Self::Access(detail),
            types::Error::Diverged(label) => Self::Diverged(label),
            types::Error::Other(detail) => Self::Other(detail),
        }
    }
}
