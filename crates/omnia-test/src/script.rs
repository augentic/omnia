//! The FIFO script every scripted double adapts.
//!
//! A [`Script`] holds an ordered queue of turns and records every request
//! that consumed one. Consuming past the end panics unless the script opted
//! into a fallback with [`Script::then`]; leaving turns unconsumed fails the
//! test either explicitly ([`Script::assert_exhausted`]) or when the last
//! handle drops. Clones share one queue, so a scenario and the provider it
//! hands to the code under test observe the same state.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::{fmt, thread};

type Fallback<Turn> = Box<dyn Fn() -> Turn + Send + Sync>;

/// A shared FIFO of scripted turns that records the requests consuming them.
///
/// ```
/// use omnia_test::Script;
///
/// let script: Script<&str, u32> = Script::new([1, 2]).then(|| 0);
/// assert_eq!(script.next("a"), 1);
/// assert_eq!(script.next("b"), 2);
/// assert_eq!(script.next("c"), 0, "past the script, the fallback answers");
/// assert_eq!(script.seen(), ["a", "b", "c"]);
/// ```
pub struct Script<Req, Turn> {
    inner: Arc<Inner<Req, Turn>>,
}

struct Inner<Req, Turn> {
    turns: Mutex<VecDeque<Turn>>,
    seen: Mutex<Vec<Req>>,
    fallback: OnceLock<Fallback<Turn>>,
}

impl<Req, Turn> Clone for Script<Req, Turn> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<Req, Turn> fmt::Debug for Script<Req, Turn> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Script")
            .field("remaining", &self.remaining())
            .field("seen", &self.inner.seen.lock().map_or(0, |seen| seen.len()))
            .field("soft", &self.inner.fallback.get().is_some())
            .finish()
    }
}

impl<Req, Turn> Script<Req, Turn> {
    /// A script of ordered turns.
    pub fn new(turns: impl IntoIterator<Item = Turn>) -> Self {
        Self {
            inner: Arc::new(Inner {
                turns: Mutex::new(turns.into_iter().collect()),
                seen: Mutex::new(Vec::new()),
                fallback: OnceLock::new(),
            }),
        }
    }

    /// Answers every request past the scripted turns with `fallback` instead
    /// of panicking — for suites that exercise the code under test's own
    /// handling of an exhausted or failing dependency.
    ///
    /// # Panics
    ///
    /// Panics if a fallback was already set.
    #[must_use]
    pub fn then(self, fallback: impl Fn() -> Turn + Send + Sync + 'static) -> Self {
        assert!(
            self.inner.fallback.set(Box::new(fallback)).is_ok(),
            "script already has a fallback"
        );
        self
    }

    /// Edits the unconsumed turn at `index` in place.
    ///
    /// # Panics
    ///
    /// Panics when no turn is scripted at `index`, or if a lock is poisoned.
    #[must_use]
    #[track_caller]
    pub fn edit(self, index: usize, edit: impl FnOnce(&mut Turn)) -> Self {
        let mut turns = self.inner.turns.lock().expect("script lock");
        let len = turns.len();
        let turn = turns
            .get_mut(index)
            .unwrap_or_else(|| panic!("no scripted turn at index {index} ({len} scripted)"));
        edit(turn);
        drop(turns);
        self
    }

    /// Records `request` and pops the next turn.
    ///
    /// # Panics
    ///
    /// Panics when the script is exhausted and no fallback was set with
    /// [`Script::then`], or if a lock is poisoned.
    #[track_caller]
    pub fn next(&self, request: Req) -> Turn {
        self.try_next(request).unwrap_or_else(|| {
            let consumed = self.inner.seen.lock().map_or(0, |seen| seen.len());
            panic!(
                "script exhausted: {} turn(s) consumed, none scripted for request #{consumed}",
                consumed - 1
            )
        })
    }

    /// Records `request` and pops the next turn, or `None` once the script
    /// is exhausted and no fallback was set — for doubles whose contract is
    /// to fail the caller rather than the test thread.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    pub fn try_next(&self, request: Req) -> Option<Turn> {
        self.inner.seen.lock().expect("seen lock").push(request);
        let popped = self.inner.turns.lock().expect("script lock").pop_front();
        popped.or_else(|| self.inner.fallback.get().map(|fallback| fallback()))
    }

    /// Every recorded request, in call order.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn seen(&self) -> Vec<Req>
    where
        Req: Clone,
    {
        self.inner.seen.lock().expect("seen lock").clone()
    }

    /// The number of turns not yet consumed.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.inner.turns.lock().expect("script lock").len()
    }

    /// Asserts that every scripted turn was consumed.
    ///
    /// # Panics
    ///
    /// Panics naming the number of unconsumed turns, or if a lock is poisoned.
    #[track_caller]
    pub fn assert_exhausted(&self) {
        let left = self.remaining();
        assert_eq!(left, 0, "script has {left} unconsumed turn(s)");
    }
}

impl<Req, Turn> Drop for Inner<Req, Turn> {
    // The last handle dropping with turns left is a forgotten assertion; it
    // fails the test unless a panic is already unwinding, which it must not
    // mask (a panic during unwinding aborts the process).
    fn drop(&mut self) {
        if thread::panicking() {
            return;
        }
        let left = self.turns.get_mut().map_or(0, |turns| turns.len());
        assert!(
            left == 0,
            "script dropped with {left} unconsumed turn(s); assert_exhausted() names them earlier"
        );
    }
}

/// One recorded model request, as both rungs see it.
///
/// The union of what guest and host scenarios assert on: the guest
/// `Scripted` and the host `ScriptedModel` project their own request types
/// into this record so a scenario reads identically wherever it runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The system prompt.
    pub system: Option<String>,
    /// Message bodies in turn order.
    pub messages: Vec<String>,
    /// The requested output shape.
    pub format: SeenFormat,
    /// Declared function-tool names, in declaration order.
    pub tools: Vec<String>,
    /// The workspace lent to the model, if any.
    pub workspace: Option<String>,
}

/// The output shape a recorded request asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SeenFormat {
    /// Plain text.
    #[default]
    Text,
    /// A JSON object.
    Json,
    /// JSON constrained by a named schema.
    Schema {
        /// The schema name passed to the provider.
        name: String,
        /// The JSON Schema document.
        schema: String,
    },
}

/// One tool call a scripted model drove, and the outcome the code under test
/// answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exchange {
    /// Tool name.
    pub tool: String,
    /// JSON arguments as sent.
    pub arguments: String,
    /// The answer; `Err` is the tool's model-visible failure text.
    pub outcome: Result<String, String>,
}
