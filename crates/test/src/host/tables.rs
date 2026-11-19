//! A predicate-matched `omnia:sql` backend recording every statement.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use futures::FutureExt as _;
use futures::future::{BoxFuture, ready};
use omnia_wasi_sql::{Connection, Error, FutureResult, Row, SqlClient, Statement, Value};

/// A statement predicate over the SQL text and its bound parameters.
pub type Predicate = Arc<dyn Fn(&str, &[Value]) -> bool + Send + Sync>;

/// One statement a guest ran.
#[derive(Clone, Debug)]
pub struct Executed {
    /// The database the guest opened it on.
    pub binding: String,
    /// The SQL text.
    pub sql: String,
    /// The bound parameters.
    pub params: Vec<Value>,
}

/// One transaction a guest began.
#[derive(Clone, Debug)]
pub struct Transacted {
    /// The database it began on.
    pub binding: String,
    /// Its statements, in the order run.
    pub statements: Vec<Executed>,
    /// How it ended, or `None` while it is open.
    pub ended: Option<Ended>,
}

/// How a [`Transacted`] ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The guest committed it.
    Committed,
    /// The guest rolled it back, or the host did once the guest dropped it
    /// or a statement in it failed.
    RolledBack,
}

/// An `omnia:sql` backend answering each statement from the first rule that
/// matches it.
///
/// - A `query` is answered by the [`on_query`](Self::on_query) rules.
/// - An `execute`, and each statement of a `batch` in order, by the
///   [`on_exec`](Self::on_exec) rules.
/// - A statement a [`failing_on`](Self::failing_on) rule matches fails with
///   its error before any other rule is tried.
///
/// Every statement is recorded with the database it ran on, and every
/// transaction a guest begins is listed. A statement no rule matches fails
/// with [`Error::Invalid`] naming it, so the guest's reply carries it to the
/// assertion. Every database name opens. Clones share one set of rules and
/// one record.
///
/// [`Backends::sql`](super::Backends::sql) serves it to a deployment.
#[derive(Clone, Default)]
pub struct ScriptedTables {
    inner: Arc<Inner>,
}

impl ScriptedTables {
    /// Answers every `query` whose SQL and parameters satisfy `matches`
    /// with `rows`.
    #[must_use]
    pub fn on_query(
        self, matches: impl Fn(&str, &[Value]) -> bool + Send + Sync + 'static, rows: Vec<Row>,
    ) -> Self {
        lock(&self.inner.queries).push(Rule::new(matches, rows));
        self
    }

    /// Answers every `execute`, and every `batch` statement, whose SQL and
    /// parameters satisfy `matches` with `affected` rows.
    #[must_use]
    pub fn on_exec(
        self, matches: impl Fn(&str, &[Value]) -> bool + Send + Sync + 'static, affected: u64,
    ) -> Self {
        lock(&self.inner.execs).push(Rule::new(matches, affected));
        self
    }

    /// Fails every statement whose SQL and parameters satisfy `matches` with
    /// `error`.
    #[must_use]
    pub fn failing_on(
        self, matches: impl Fn(&str, &[Value]) -> bool + Send + Sync + 'static, error: Error,
    ) -> Self {
        lock(&self.inner.failures).push(Rule::new(matches, error));
        self
    }

    /// Every statement run, in the order run.
    #[must_use]
    pub fn statements(&self) -> Vec<Executed> {
        lock(&self.inner.statements).clone()
    }

    /// Every transaction begun, in the order begun.
    #[must_use]
    pub fn transactions(&self) -> Vec<Transacted> {
        lock(&self.inner.transactions).clone()
    }
}

impl SqlClient for ScriptedTables {
    fn open(&self, name: String) -> FutureResult<Arc<dyn Connection>> {
        let session: Arc<dyn Connection> = Arc::new(Session {
            tables: self.clone(),
            binding: name,
            transaction: Mutex::default(),
        });
        ready(Ok(session)).boxed()
    }
}

impl fmt::Debug for ScriptedTables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptedTables")
            .field("statements", &self.statements())
            .field("transactions", &self.transactions())
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Inner {
    queries: Mutex<Vec<Rule<Vec<Row>>>>,
    execs: Mutex<Vec<Rule<u64>>>,
    failures: Mutex<Vec<Rule<Error>>>,
    statements: Mutex<Vec<Executed>>,
    transactions: Mutex<Vec<Transacted>>,
}

struct Rule<T> {
    matches: Predicate,
    outcome: T,
}

impl<T: Clone> Rule<T> {
    fn new(matches: impl Fn(&str, &[Value]) -> bool + Send + Sync + 'static, outcome: T) -> Self {
        Self {
            matches: Arc::new(matches),
            outcome,
        }
    }

    fn first(rules: &Mutex<Vec<Self>>, executed: &Executed) -> Option<T> {
        lock(rules)
            .iter()
            .find(|rule| (rule.matches)(&executed.sql, &executed.params))
            .map(|rule| rule.outcome.clone())
    }
}

// One connection the host opened. The host runs a transaction's statements
// on the connection that began it, so the open transaction is the
// session's own.
struct Session {
    tables: ScriptedTables,
    binding: String,
    transaction: Mutex<Option<usize>>,
}

impl Session {
    fn record(&self, statement: Statement) -> Executed {
        let executed = Executed {
            binding: self.binding.clone(),
            sql: statement.sql,
            params: statement.params,
        };
        lock(&self.tables.inner.statements).push(executed.clone());
        let transaction = *lock(&self.transaction);
        if let Some(index) = transaction {
            lock(&self.tables.inner.transactions)[index].statements.push(executed.clone());
        }
        executed
    }

    fn answer<T: Clone>(
        &self, rules: &Mutex<Vec<Rule<T>>>, statement: Statement,
    ) -> Result<T, Error> {
        let executed = self.record(statement);
        if let Some(error) = Rule::first(&self.tables.inner.failures, &executed) {
            return Err(error);
        }
        Rule::first(rules, &executed).ok_or_else(|| {
            Error::Invalid(format!("no rule matches `{}` with {:?}", executed.sql, executed.params))
        })
    }

    fn end(&self, ended: Ended) {
        let transaction = lock(&self.transaction).take();
        if let Some(index) = transaction {
            lock(&self.tables.inner.transactions)[index].ended.get_or_insert(ended);
        }
    }
}

impl Connection for Session {
    fn query(&self, sql: String, params: Vec<Value>) -> FutureResult<Vec<Row>> {
        let rows = self.answer(&self.tables.inner.queries, Statement { sql, params });
        settle(rows)
    }

    fn execute(&self, sql: String, params: Vec<Value>) -> FutureResult<u64> {
        let affected = self.answer(&self.tables.inner.execs, Statement { sql, params });
        settle(affected)
    }

    fn batch(&self, statements: Vec<Statement>) -> FutureResult<Vec<u64>> {
        let affected = statements
            .into_iter()
            .map(|statement| self.answer(&self.tables.inner.execs, statement))
            .collect();
        settle(affected)
    }

    fn begin(&self) -> FutureResult<()> {
        let mut transactions = lock(&self.tables.inner.transactions);
        *lock(&self.transaction) = Some(transactions.len());
        transactions.push(Transacted {
            binding: self.binding.clone(),
            statements: Vec::new(),
            ended: None,
        });
        drop(transactions);
        settle(Ok(()))
    }

    fn commit(&self) -> FutureResult<()> {
        self.end(Ended::Committed);
        settle(Ok(()))
    }

    fn rollback(&self) -> FutureResult<()> {
        self.end(Ended::RolledBack);
        settle(Ok(()))
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").field("binding", &self.binding).finish_non_exhaustive()
    }
}

fn settle<T: Send + 'static>(outcome: Result<T, Error>) -> BoxFuture<'static, anyhow::Result<T>> {
    ready(outcome.map_err(anyhow::Error::from)).boxed()
}

// a panicking assertion elsewhere must not hide what the run recorded
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
