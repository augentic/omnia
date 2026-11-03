//! A FIFO-scripted `VcsClient` recording every version-control call.

use std::path::PathBuf;

use futures::FutureExt as _;
use omnia_backend::FutureResult;
use omnia_backend::vcs::{self, Change, CloneOptions, Entry, Merged, Place, Rule, VcsClient};

use crate::Script;

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

/// A FIFO-scripted [`VcsClient`] recording every operation and its resolved path.
#[derive(Clone, Debug)]
pub struct ScriptedVcs {
    script: Script<SeenVcs, Result<VcsTurn, vcs::Error>>,
}

impl Default for ScriptedVcs {
    fn default() -> Self {
        Self::new([])
    }
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

    /// Asserts that every scripted turn was consumed and no call ran past the end.
    ///
    /// # Panics
    ///
    /// Panics naming the number of unconsumed turns or overruns.
    #[track_caller]
    pub fn assert_exhausted(&self) {
        self.script.assert_exhausted();
    }

    fn pop(&self, op: &'static str, place: &Place, args: Vec<String>) -> anyhow::Result<VcsTurn> {
        let seen = SeenVcs {
            op,
            path: place.path().to_path_buf(),
            args,
        };
        let turn = self.script.try_next(seen).ok_or_else(|| {
            let consumed = self.script.seen().len();
            anyhow::anyhow!(
                "vcs script exhausted at `{op}`: {} turn(s) consumed, none scripted for request \
                 #{consumed}",
                consumed - 1
            )
        })?;
        turn.map_err(Into::into)
    }
}

impl VcsClient for ScriptedVcs {
    fn resolve(&self, repo: Place, revision: String) -> FutureResult<String> {
        let res = self.pop("resolve", &repo, vec![revision]).and_then(|turn| match turn {
            VcsTurn::Revision(sha) => Ok(sha),
            other => Err(anyhow::anyhow!("expected VcsTurn::Revision for resolve, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn descends(&self, repo: Place, ancestor: String, descendant: String) -> FutureResult<bool> {
        let res =
            self.pop("descends", &repo, vec![ancestor, descendant]).and_then(|turn| match turn {
                VcsTurn::Bool(b) => Ok(b),
                other => Err(anyhow::anyhow!("expected VcsTurn::Bool for descends, got {other:?}")),
            });
        async move { res }.boxed()
    }

    fn head(&self, at: Place) -> FutureResult<String> {
        let res = self.pop("head", &at, vec![]).and_then(|turn| match turn {
            VcsTurn::Revision(sha) => Ok(sha),
            other => Err(anyhow::anyhow!("expected VcsTurn::Revision for head, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn commit(&self, at: Place, message: String) -> FutureResult<Option<String>> {
        let res = self.pop("commit", &at, vec![message]).and_then(|turn| match turn {
            VcsTurn::Commit(sha) => Ok(sha),
            other => Err(anyhow::anyhow!("expected VcsTurn::Commit for commit, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn merge(
        &self, at: Place, revision: String, message: String, _policy: Vec<Rule>,
    ) -> FutureResult<Merged> {
        let res = self.pop("merge", &at, vec![revision, message]).and_then(|turn| match turn {
            VcsTurn::Merged(merged) => Ok(merged),
            other => Err(anyhow::anyhow!("expected VcsTurn::Merged for merge, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn log(&self, repo: Place, revision: String, base: String) -> FutureResult<Vec<Entry>> {
        let res = self.pop("log", &repo, vec![revision, base]).and_then(|turn| match turn {
            VcsTurn::Log(entries) => Ok(entries),
            other => Err(anyhow::anyhow!("expected VcsTurn::Log for log, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn init(&self, at: Place) -> FutureResult<()> {
        let res = self.pop("init", &at, vec![]).and_then(|turn| match turn {
            VcsTurn::Unit => Ok(()),
            other => Err(anyhow::anyhow!("expected VcsTurn::Unit for init, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn add(&self, repo: Place, at: Place, revision: String) -> FutureResult<()> {
        let res = self.pop("add", &repo, vec![at.path().display().to_string(), revision]).and_then(
            |turn| match turn {
                VcsTurn::Unit => Ok(()),
                other => Err(anyhow::anyhow!("expected VcsTurn::Unit for add, got {other:?}")),
            },
        );
        async move { res }.boxed()
    }

    fn remove(&self, at: Place) -> FutureResult<()> {
        let res = self.pop("remove", &at, vec![]).and_then(|turn| match turn {
            VcsTurn::Unit => Ok(()),
            other => Err(anyhow::anyhow!("expected VcsTurn::Unit for remove, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn pending(&self, at: Place) -> FutureResult<Vec<Change>> {
        let res = self.pop("pending", &at, vec![]).and_then(|turn| match turn {
            VcsTurn::Pending(changes) => Ok(changes),
            other => Err(anyhow::anyhow!("expected VcsTurn::Pending for pending, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn clone_repo(&self, url: String, at: Place, options: CloneOptions) -> FutureResult<()> {
        let res = self.pop("clone_repo", &at, vec![url, format!("{:?}", options.depth)]).and_then(
            |turn| match turn {
                VcsTurn::Unit => Ok(()),
                other => {
                    Err(anyhow::anyhow!("expected VcsTurn::Unit for clone_repo, got {other:?}"))
                }
            },
        );
        async move { res }.boxed()
    }

    fn fetch(&self, repo: Place, remote: String) -> FutureResult<()> {
        let res = self.pop("fetch", &repo, vec![remote]).and_then(|turn| match turn {
            VcsTurn::Unit => Ok(()),
            other => Err(anyhow::anyhow!("expected VcsTurn::Unit for fetch, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn label(&self, repo: Place, name: String, revision: String) -> FutureResult<()> {
        let res = self.pop("label", &repo, vec![name, revision]).and_then(|turn| match turn {
            VcsTurn::Unit => Ok(()),
            other => Err(anyhow::anyhow!("expected VcsTurn::Unit for label, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn labelled(&self, repo: Place, name: String) -> FutureResult<String> {
        let res = self.pop("labelled", &repo, vec![name]).and_then(|turn| match turn {
            VcsTurn::Revision(sha) => Ok(sha),
            other => Err(anyhow::anyhow!("expected VcsTurn::Revision for labelled, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn fetched(&self, repo: Place, remote: String, name: String) -> FutureResult<String> {
        let res = self.pop("fetched", &repo, vec![remote, name]).and_then(|turn| match turn {
            VcsTurn::Revision(sha) => Ok(sha),
            other => Err(anyhow::anyhow!("expected VcsTurn::Revision for fetched, got {other:?}")),
        });
        async move { res }.boxed()
    }

    fn push(&self, repo: Place, remote: String, label: String) -> FutureResult<()> {
        let res = self.pop("push", &repo, vec![remote, label]).and_then(|turn| match turn {
            VcsTurn::Unit => Ok(()),
            other => Err(anyhow::anyhow!("expected VcsTurn::Unit for push, got {other:?}")),
        });
        async move { res }.boxed()
    }
}
