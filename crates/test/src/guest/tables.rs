//! A predicate-matched `SqlDb` recording every statement.

use std::fmt;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use omnia_sdk::sql::{DEFAULT, Row, Statement, Transaction, Value};
use omnia_sdk::{Result, SqlDb};

/// A statement predicate over the SQL text and its bound parameters.
pub type Predicate = Arc<dyn Fn(&str, &[Value]) -> bool + Send + Sync>;

/// One statement the code under test executed.
#[derive(Clone, Debug)]
pub struct Executed {
    /// The database binding the statement ran on.
    pub binding: String,
    /// The SQL text.
    pub sql: String,
    /// The bound parameters.
    pub params: Vec<Value>,
}

/// One transaction the code under test began.
#[derive(Clone, Debug)]
pub struct Transacted {
    /// The database binding it began on.
    pub binding: String,
    /// Its statements, in call order.
    pub statements: Vec<Executed>,
    /// How it ended, or `None` while it is open.
    pub ended: Option<Ended>,
}

/// How a [`Transacted`] ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// [`Transaction::commit`] ended it.
    Committed,
    /// [`Transaction::rollback`] ended it, or dropping it did.
    RolledBack,
}

/// SQL `query`, `execute`, and `batch` answered by the first matching
/// predicate; every statement is recorded and an unmatched one panics naming it.
///
/// A handle runs on the binding [`DEFAULT`] until
/// [`named`](SqlDb::named) returns one on another binding, and every
/// [`Executed`] records the binding it ran on. A `batch` answers each
/// statement through the `on_exec` rules, in order.
///
/// [`begin`](SqlDb::begin) returns a [`ScriptedTransaction`] answered by the
/// same rules, and [`transactions`](Self::transactions) lists each one begun.
///
/// ```
/// use omnia_sdk::SqlDb as _;
/// use omnia_sdk::sql::Value;
/// use omnia_test::guest::ScriptedTables;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let tables = ScriptedTables::default()
///     .on_exec(|sql, params| sql.starts_with("INSERT") && params.len() == 1, 1);
/// let params = [Value::from("ann")];
/// assert_eq!(tables.execute("INSERT INTO t VALUES (?)", &params).await.unwrap(), 1);
/// assert_eq!(tables.statements()[0].sql, "INSERT INTO t VALUES (?)");
/// assert_eq!(tables.statements()[0].binding, "default");
/// # });
/// ```
#[derive(Clone)]
pub struct ScriptedTables {
    inner: Arc<Inner>,
    binding: Arc<str>,
}

impl Default for ScriptedTables {
    fn default() -> Self {
        Self {
            inner: Arc::default(),
            binding: Arc::from(DEFAULT),
        }
    }
}

impl ScriptedTables {
    /// Answers every `query` whose SQL and parameters satisfy `matches`
    /// with `rows`.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn on_query(
        self, matches: impl Fn(&str, &[Value]) -> bool + Send + Sync + 'static, rows: Vec<Row>,
    ) -> Self {
        self.inner.queries.lock().expect("queries lock").push(Rule {
            matches: Arc::new(matches),
            outcome: rows,
        });
        self
    }

    /// Answers every `execute`, and every `batch` statement, whose SQL and
    /// parameters satisfy `matches` with `affected` rows.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn on_exec(
        self, matches: impl Fn(&str, &[Value]) -> bool + Send + Sync + 'static, affected: u64,
    ) -> Self {
        self.inner.execs.lock().expect("execs lock").push(Rule {
            matches: Arc::new(matches),
            outcome: affected,
        });
        self
    }

    /// Every statement executed, in call order.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn statements(&self) -> Vec<Executed> {
        self.inner.statements.lock().expect("statements lock").clone()
    }

    /// Every transaction begun, in the order begun.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn transactions(&self) -> Vec<Transacted> {
        self.inner.transactions.lock().expect("transactions lock").clone()
    }

    fn record(&self, sql: &str, params: &[Value]) -> Executed {
        let executed = Executed {
            binding: self.binding.to_string(),
            sql: sql.to_owned(),
            params: params.to_vec(),
        };
        self.inner.statements.lock().expect("statements lock").push(executed.clone());
        executed
    }

    fn rows(&self, executed: &Executed) -> Vec<Row> {
        Self::first_match(&self.inner.queries, executed).unwrap_or_else(|| {
            panic!("no on_query rule matches `{}` with {:?}", executed.sql, executed.params)
        })
    }

    fn affected(&self, executed: &Executed) -> u64 {
        Self::first_match(&self.inner.execs, executed).unwrap_or_else(|| {
            panic!("no on_exec rule matches `{}` with {:?}", executed.sql, executed.params)
        })
    }

    fn first_match<T: Clone>(rules: &Mutex<Vec<Rule<T>>>, executed: &Executed) -> Option<T> {
        rules
            .lock()
            .expect("rules lock")
            .iter()
            .find(|rule| (rule.matches)(&executed.sql, &executed.params))
            .map(|rule| rule.outcome.clone())
    }
}

impl fmt::Debug for ScriptedTables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptedTables")
            .field("binding", &self.binding)
            .field("queries", &self.inner.queries.lock().map_or(0, |q| q.len()))
            .field("execs", &self.inner.execs.lock().map_or(0, |e| e.len()))
            .field("statements", &self.statements())
            .field("transactions", &self.transactions())
            .finish()
    }
}

// panics when no rule matches the statement
impl SqlDb for ScriptedTables {
    type Named = Self;
    type Transaction = ScriptedTransaction;

    fn query(&self, sql: &str, params: &[Value]) -> impl Future<Output = Result<Vec<Row>>> + Send {
        ready(Ok(self.rows(&self.record(sql, params))))
    }

    fn execute(&self, sql: &str, params: &[Value]) -> impl Future<Output = Result<u64>> + Send {
        ready(Ok(self.affected(&self.record(sql, params))))
    }

    fn batch(&self, statements: &[Statement]) -> impl Future<Output = Result<Vec<u64>>> + Send {
        let affected = statements
            .iter()
            .map(|statement| self.affected(&self.record(&statement.sql, &statement.params)))
            .collect();
        ready(Ok(affected))
    }

    fn begin(&self) -> impl Future<Output = Result<ScriptedTransaction>> + Send {
        ready(Ok(ScriptedTransaction::new(self.clone())))
    }

    fn named(&self, name: &str) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            binding: Arc::from(name),
        }
    }
}

/// A transaction on a [`ScriptedTables`], answered by the same rules.
///
/// Each statement joins [`ScriptedTables::statements`] and the transaction's
/// own [`Transacted`]. Dropping it before it ends records
/// [`Ended::RolledBack`].
#[derive(Debug)]
pub struct ScriptedTransaction {
    tables: ScriptedTables,
    // its place in `Inner::transactions`
    index: usize,
}

impl ScriptedTransaction {
    fn new(tables: ScriptedTables) -> Self {
        let mut transactions = tables.inner.transactions.lock().expect("transactions lock");
        let index = transactions.len();
        transactions.push(Transacted {
            binding: tables.binding.to_string(),
            statements: Vec::new(),
            ended: None,
        });
        drop(transactions);
        Self { tables, index }
    }

    fn record(&self, sql: &str, params: &[Value]) -> Executed {
        let executed = self.tables.record(sql, params);
        let mut transactions = self.tables.inner.transactions.lock().expect("transactions lock");
        transactions[self.index].statements.push(executed.clone());
        executed
    }

    fn end(&self, ended: Ended) {
        let mut transactions = self.tables.inner.transactions.lock().expect("transactions lock");
        transactions[self.index].ended.get_or_insert(ended);
    }
}

// panics when no rule matches the statement
impl Transaction for ScriptedTransaction {
    fn query(&self, sql: &str, params: &[Value]) -> impl Future<Output = Result<Vec<Row>>> + Send {
        ready(Ok(self.tables.rows(&self.record(sql, params))))
    }

    fn execute(&self, sql: &str, params: &[Value]) -> impl Future<Output = Result<u64>> + Send {
        ready(Ok(self.tables.affected(&self.record(sql, params))))
    }

    fn commit(self) -> impl Future<Output = Result<()>> + Send {
        self.end(Ended::Committed);
        ready(Ok(()))
    }

    fn rollback(self) -> impl Future<Output = Result<()>> + Send {
        self.end(Ended::RolledBack);
        ready(Ok(()))
    }
}

impl Drop for ScriptedTransaction {
    fn drop(&mut self) {
        // a drop must not panic, so a poisoned lock records nothing
        if let Ok(mut transactions) = self.tables.inner.transactions.lock() {
            transactions[self.index].ended.get_or_insert(Ended::RolledBack);
        }
    }
}

#[derive(Default)]
struct Inner {
    queries: Mutex<Vec<Rule<Vec<Row>>>>,
    execs: Mutex<Vec<Rule<u64>>>,
    statements: Mutex<Vec<Executed>>,
    transactions: Mutex<Vec<Transacted>>,
}

struct Rule<T> {
    matches: Predicate,
    outcome: T,
}
