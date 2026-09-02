//! A predicate-matched `TableStore` recording every statement.

use std::fmt;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use omnia_guest::TableStore;
use omnia_wasi_sql::{DataType, Row};

/// A statement predicate over the SQL text and its bound parameters.
pub type Predicate = Arc<dyn Fn(&str, &[DataType]) -> bool + Send + Sync>;

/// One statement the code under test issued.
#[derive(Clone, Debug)]
pub struct Statement {
    /// The connection name.
    pub connection: String,
    /// The SQL text.
    pub sql: String,
    /// The bound parameters.
    pub params: Vec<DataType>,
}

struct Rule<T> {
    matches: Predicate,
    outcome: T,
}

#[derive(Default)]
struct Inner {
    queries: Mutex<Vec<Rule<Vec<Row>>>>,
    execs: Mutex<Vec<Rule<u32>>>,
    statements: Mutex<Vec<Statement>>,
}

/// SQL `query`/`exec` answered by the first matching predicate; every
/// statement is recorded and an unmatched one panics naming it.
///
/// ```
/// use omnia_guest::TableStore as _;
/// use omnia_test::guest::ScriptedTables;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let tables = ScriptedTables::default()
///     .on_exec(|sql, params| sql.starts_with("INSERT") && params.len() == 1, 1);
/// let params = vec![omnia_wasi_sql::DataType::Str(Some("ann".into()))];
/// assert_eq!(
///     tables.exec("db".into(), "INSERT INTO t VALUES (?)".into(), params).await.unwrap(),
///     1
/// );
/// assert_eq!(tables.statements()[0].sql, "INSERT INTO t VALUES (?)");
/// # });
/// ```
#[derive(Clone, Default)]
pub struct ScriptedTables {
    inner: Arc<Inner>,
}

impl fmt::Debug for ScriptedTables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptedTables")
            .field("queries", &self.inner.queries.lock().map_or(0, |q| q.len()))
            .field("execs", &self.inner.execs.lock().map_or(0, |e| e.len()))
            .field("statements", &self.statements())
            .finish()
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
        self, matches: impl Fn(&str, &[DataType]) -> bool + Send + Sync + 'static, rows: Vec<Row>,
    ) -> Self {
        self.inner.queries.lock().expect("queries lock").push(Rule {
            matches: Arc::new(matches),
            outcome: rows,
        });
        self
    }

    /// Answers every `exec` whose SQL and parameters satisfy `matches` with
    /// `affected` rows.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn on_exec(
        self, matches: impl Fn(&str, &[DataType]) -> bool + Send + Sync + 'static, affected: u32,
    ) -> Self {
        self.inner.execs.lock().expect("execs lock").push(Rule {
            matches: Arc::new(matches),
            outcome: affected,
        });
        self
    }

    /// Every statement issued, in call order.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn statements(&self) -> Vec<Statement> {
        self.inner.statements.lock().expect("statements lock").clone()
    }

    fn record(&self, connection: String, sql: String, params: Vec<DataType>) -> Statement {
        let statement = Statement {
            connection,
            sql,
            params,
        };
        self.inner.statements.lock().expect("statements lock").push(statement.clone());
        statement
    }

    fn first_match<T: Clone>(rules: &Mutex<Vec<Rule<T>>>, statement: &Statement) -> Option<T> {
        rules
            .lock()
            .expect("rules lock")
            .iter()
            .find(|rule| (rule.matches)(&statement.sql, &statement.params))
            .map(|rule| rule.outcome.clone())
    }
}

impl TableStore for ScriptedTables {
    /// # Panics
    ///
    /// Panics when no `on_query` rule matches the statement.
    fn query(
        &self, conn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<Vec<Row>>> + Send {
        let statement = self.record(conn_name, query, params);
        let rows = Self::first_match(&self.inner.queries, &statement).unwrap_or_else(|| {
            panic!("no on_query rule matches `{}` with {:?}", statement.sql, statement.params)
        });
        ready(Ok(rows))
    }

    /// # Panics
    ///
    /// Panics when no `on_exec` rule matches the statement.
    fn exec(
        &self, conn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<u32>> + Send {
        let statement = self.record(conn_name, query, params);
        let affected = Self::first_match(&self.inner.execs, &statement).unwrap_or_else(|| {
            panic!("no on_exec rule matches `{}` with {:?}", statement.sql, statement.params)
        });
        ready(Ok(affected))
    }
}
