//! A FIFO-scripted `Vcs` double for handler-level tests.

use std::future::{Future, ready};
use std::path::PathBuf;

use omnia_sdk::vcs::{self, Change, CloneOptions, Entry, Merged, Rule, Vcs};

use crate::Script;

/// A FIFO-scripted [`Vcs`] capability double recording every call and raw path.
#[derive(Clone, Debug)]
pub struct ScriptedVcs {
    script: Script<SeenVcs, Result<VcsTurn, vcs::Error>>,
}

impl ScriptedVcs {
    /// A script of ordered version-control turn results.
    pub fn new(turns: impl IntoIterator<Item = Result<VcsTurn, vcs::Error>>) -> Self {
        Self {
            script: Script::new(turns),
        }
    }

    /// A script of successful [`VcsTurn::Revision`] answers.
    pub fn resolving<S: Into<String>>(revisions: impl IntoIterator<Item = S>) -> Self {
        Self::new(revisions.into_iter().map(|rev| Ok(VcsTurn::Revision(rev.into()))))
    }

    /// Answers every call past the scripted turns with `fallback`.
    ///
    /// # Panics
    ///
    /// Panics if a fallback was already set.
    #[must_use]
    pub fn then(
        self, fallback: impl Fn() -> Result<VcsTurn, vcs::Error> + Send + Sync + 'static,
    ) -> Self {
        Self {
            script: self.script.then(fallback),
        }
    }

    /// Every recorded version-control call, in call order.
    #[must_use]
    pub fn seen(&self) -> Vec<SeenVcs> {
        self.script.seen()
    }

    /// Asserts that every scripted turn was consumed.
    ///
    /// # Panics
    ///
    /// Panics naming the number of unconsumed turns.
    #[track_caller]
    pub fn assert_exhausted(&self) {
        self.script.assert_exhausted();
    }

    #[track_caller]
    fn pop(&self, op: &'static str, path: &str, args: Vec<String>) -> Result<VcsTurn, vcs::Error> {
        self.script.next(SeenVcs {
            op,
            path: PathBuf::from(path),
            args,
        })
    }
}

impl Default for ScriptedVcs {
    fn default() -> Self {
        Self::new([])
    }
}

impl Vcs for ScriptedVcs {
    fn resolve(
        &self, repo: &str, revision: &str,
    ) -> impl Future<Output = Result<String, vcs::Error>> + Send {
        let res = self.pop("resolve", repo, vec![revision.to_owned()]).map(|turn| match turn {
            VcsTurn::Revision(sha) => sha,
            other => panic!("expected VcsTurn::Revision for resolve, got {other:?}"),
        });
        ready(res)
    }

    fn descends(
        &self, repo: &str, ancestor: &str, descendant: &str,
    ) -> impl Future<Output = Result<bool, vcs::Error>> + Send {
        let res = self.pop("descends", repo, vec![ancestor.to_owned(), descendant.to_owned()]).map(
            |turn| match turn {
                VcsTurn::Bool(b) => b,
                other => panic!("expected VcsTurn::Bool for descends, got {other:?}"),
            },
        );
        ready(res)
    }

    fn head(&self, at: &str) -> impl Future<Output = Result<String, vcs::Error>> + Send {
        let res = self.pop("head", at, vec![]).map(|turn| match turn {
            VcsTurn::Revision(sha) => sha,
            other => panic!("expected VcsTurn::Revision for head, got {other:?}"),
        });
        ready(res)
    }

    fn commit(
        &self, at: &str, message: &str,
    ) -> impl Future<Output = Result<Option<String>, vcs::Error>> + Send {
        let res = self.pop("commit", at, vec![message.to_owned()]).map(|turn| match turn {
            VcsTurn::Commit(sha) => sha,
            other => panic!("expected VcsTurn::Commit for commit, got {other:?}"),
        });
        ready(res)
    }

    fn merge(
        &self, at: &str, revision: &str, message: &str, _policy: &[Rule],
    ) -> impl Future<Output = Result<Merged, vcs::Error>> + Send {
        let res =
            self.pop("merge", at, vec![revision.to_owned(), message.to_owned()]).map(|turn| {
                match turn {
                    VcsTurn::Merged(merged) => merged,
                    other => panic!("expected VcsTurn::Merged for merge, got {other:?}"),
                }
            });
        ready(res)
    }

    fn log(
        &self, repo: &str, revision: &str, base: &str,
    ) -> impl Future<Output = Result<Vec<Entry>, vcs::Error>> + Send {
        let res =
            self.pop("log", repo, vec![revision.to_owned(), base.to_owned()]).map(
                |turn| match turn {
                    VcsTurn::Log(entries) => entries,
                    other => panic!("expected VcsTurn::Log for log, got {other:?}"),
                },
            );
        ready(res)
    }

    fn init(&self, at: &str) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        let res = self.pop("init", at, vec![]).map(|turn| match turn {
            VcsTurn::Unit => (),
            other => panic!("expected VcsTurn::Unit for init, got {other:?}"),
        });
        ready(res)
    }

    fn add(
        &self, repo: &str, at: &str, revision: &str,
    ) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        let res =
            self.pop("add", repo, vec![at.to_owned(), revision.to_owned()]).map(
                |turn| match turn {
                    VcsTurn::Unit => (),
                    other => panic!("expected VcsTurn::Unit for add, got {other:?}"),
                },
            );
        ready(res)
    }

    fn remove(&self, at: &str) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        let res = self.pop("remove", at, vec![]).map(|turn| match turn {
            VcsTurn::Unit => (),
            other => panic!("expected VcsTurn::Unit for remove, got {other:?}"),
        });
        ready(res)
    }

    fn pending(&self, at: &str) -> impl Future<Output = Result<Vec<Change>, vcs::Error>> + Send {
        let res = self.pop("pending", at, vec![]).map(|turn| match turn {
            VcsTurn::Pending(changes) => changes,
            other => panic!("expected VcsTurn::Pending for pending, got {other:?}"),
        });
        ready(res)
    }

    fn clone_repo(
        &self, url: &str, at: &str, options: CloneOptions,
    ) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        let res = self
            .pop("clone_repo", at, vec![url.to_owned(), format!("{:?}", options.depth)])
            .map(|turn| match turn {
                VcsTurn::Unit => (),
                other => panic!("expected VcsTurn::Unit for clone_repo, got {other:?}"),
            });
        ready(res)
    }

    fn fetch(
        &self, repo: &str, remote: &str,
    ) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        let res = self.pop("fetch", repo, vec![remote.to_owned()]).map(|turn| match turn {
            VcsTurn::Unit => (),
            other => panic!("expected VcsTurn::Unit for fetch, got {other:?}"),
        });
        ready(res)
    }

    fn label(
        &self, repo: &str, name: &str, revision: &str,
    ) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        let res = self.pop("label", repo, vec![name.to_owned(), revision.to_owned()]).map(|turn| {
            match turn {
                VcsTurn::Unit => (),
                other => panic!("expected VcsTurn::Unit for label, got {other:?}"),
            }
        });
        ready(res)
    }

    fn labelled(
        &self, repo: &str, name: &str,
    ) -> impl Future<Output = Result<String, vcs::Error>> + Send {
        let res = self.pop("labelled", repo, vec![name.to_owned()]).map(|turn| match turn {
            VcsTurn::Revision(sha) => sha,
            other => panic!("expected VcsTurn::Revision for labelled, got {other:?}"),
        });
        ready(res)
    }

    fn fetched(
        &self, repo: &str, remote: &str, name: &str,
    ) -> impl Future<Output = Result<String, vcs::Error>> + Send {
        let res = self.pop("fetched", repo, vec![remote.to_owned(), name.to_owned()]).map(|turn| {
            match turn {
                VcsTurn::Revision(sha) => sha,
                other => panic!("expected VcsTurn::Revision for fetched, got {other:?}"),
            }
        });
        ready(res)
    }

    fn push(
        &self, repo: &str, remote: &str, label: &str,
    ) -> impl Future<Output = Result<(), vcs::Error>> + Send {
        let res =
            self.pop("push", repo, vec![remote.to_owned(), label.to_owned()]).map(
                |turn| match turn {
                    VcsTurn::Unit => (),
                    other => panic!("expected VcsTurn::Unit for push, got {other:?}"),
                },
            );
        ready(res)
    }
}

/// One recorded version-control operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenVcs {
    /// Operation name (`"resolve"`, `"head"`, `"commit"`, ...).
    pub op: &'static str,
    /// Primary repository or working-copy path.
    pub path: PathBuf,
    /// Operation-specific arguments in call order.
    pub args: Vec<String>,
}

/// One scripted return value from a version-control operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VcsTurn {
    /// A commit SHA or revision string (`resolve`, `head`, `labelled`, `fetched`).
    Revision(String),
    /// A boolean answer (`descends`).
    Bool(bool),
    /// An optional commit SHA (`commit`).
    Commit(Option<String>),
    /// A merge outcome (`merge`).
    Merged(Merged),
    /// A commit log (`log`).
    Log(Vec<Entry>),
    /// Pending working-copy changes (`pending`).
    Pending(Vec<Change>),
    /// Unit completion (`init`, `add`, `remove`, `clone_repo`, `fetch`, `label`, `push`).
    Unit,
}
