//! A predicate-matched `SqlDb` recording every statement.

use std::fmt;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use omnia_sdk::SqlDb;
use omnia_sdk::capability::Result;
use omnia_sdk::sql::{DEFAULT, Row, Statement, Value};

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

/// SQL `query`, `execute`, and `batch` answered by the first matching
/// predicate; every statement is recorded and an unmatched one panics naming it.
///
/// A handle runs on the binding [`DEFAULT`] until
/// [`named`](SqlDb::named) returns one on another binding, and every
/// [`Executed`] records the binding it ran on. A `batch` answers each
/// statement through the `on_exec` rules, in order.
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

    fn record(&self, sql: &str, params: &[Value]) -> Executed {
        let executed = Executed {
            binding: self.binding.to_string(),
            sql: sql.to_owned(),
            params: params.to_vec(),
        };
        self.inner.statements.lock().expect("statements lock").push(executed.clone());
        executed
    }

    fn affected(&self, sql: &str, params: &[Value]) -> u64 {
        let executed = self.record(sql, params);
        Self::first_match(&self.inner.execs, &executed).unwrap_or_else(|| {
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
            .finish()
    }
}

// panics when no rule matches the statement
impl SqlDb for ScriptedTables {
    type Named = Self;

    fn query(&self, sql: &str, params: &[Value]) -> impl Future<Output = Result<Vec<Row>>> + Send {
        let executed = self.record(sql, params);
        let rows = Self::first_match(&self.inner.queries, &executed).unwrap_or_else(|| {
            panic!("no on_query rule matches `{}` with {:?}", executed.sql, executed.params)
        });
        ready(Ok(rows))
    }

    fn execute(&self, sql: &str, params: &[Value]) -> impl Future<Output = Result<u64>> + Send {
        ready(Ok(self.affected(sql, params)))
    }

    fn batch(&self, statements: &[Statement]) -> impl Future<Output = Result<Vec<u64>>> + Send {
        let affected = statements
            .iter()
            .map(|statement| self.affected(&statement.sql, &statement.params))
            .collect();
        ready(Ok(affected))
    }

    fn named(&self, name: &str) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            binding: Arc::from(name),
        }
    }
}

#[derive(Default)]
struct Inner {
    queries: Mutex<Vec<Rule<Vec<Row>>>>,
    execs: Mutex<Vec<Rule<u64>>>,
    statements: Mutex<Vec<Executed>>,
}

struct Rule<T> {
    matches: Predicate,
    outcome: T,
}
