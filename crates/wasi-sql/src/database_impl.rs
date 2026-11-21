mod value;

use std::sync::Arc;

use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, FutureReader, Resource, StreamReader};

use crate::generated::omnia::sql::database::{
    self as wit, Host, HostDatabase, HostDatabaseWithStore,
};
use crate::generated::omnia::sql::types::{self, Value};
use crate::{Connection, Error, Result, ResultSet, SqlClient, Statement, WasiSql};

// The first keywords of a statement that starts or ends a transaction or a
// savepoint, which only `transactions` may do. `PREPARE` is refused only
// before `TRANSACTION`.
const TRANSACTION_CONTROL: [&str; 8] =
    ["ABORT", "BEGIN", "COMMIT", "END", "RELEASE", "ROLLBACK", "SAVEPOINT", "START"];

// The host's side of a guest's `database` handle. It keeps the connection
// `store.open` proved the name with, for the first call or `begin` to take.
#[derive(Debug)]
pub struct DatabaseHandle {
    name: String,
    opened: Option<Arc<dyn Connection>>,
}

impl DatabaseHandle {
    pub fn push<T>(
        accessor: &Accessor<T, WasiSql>, name: String, opened: Arc<dyn Connection>,
    ) -> Result<Resource<Self>> {
        let handle = Self {
            name,
            opened: Some(opened),
        };
        Ok(accessor.with(|mut store| store.get().table.push(handle))?)
    }

    // A call or `begin` after the first opens a connection of its own, so a
    // handle holds no session once it has run a call.
    pub async fn connection<T>(
        accessor: &Accessor<T, WasiSql>, resource: &Resource<Self>,
    ) -> Result<Arc<dyn Connection>> {
        let (name, opened) = accessor.with(|mut store| {
            store
                .get()
                .table
                .get_mut(resource)
                .map(|handle| (handle.name.clone(), handle.opened.take()))
        })?;
        if let Some(connection) = opened {
            return Ok(connection);
        }
        Ok(accessor.with(|mut store| store.get().client.open(name)).await?)
    }
}

impl<T> HostDatabaseWithStore<T> for WasiSql {
    async fn query(
        accessor: &Accessor<T, Self>, self_: Resource<DatabaseHandle>, sql: String,
        params: Vec<Option<Value>>,
    ) -> Result<types::ResultSet> {
        let statement = statement(sql, params)?;
        let connection = DatabaseHandle::connection(accessor, &self_).await?;
        let selected = connection.select(statement.sql, statement.params).await?;
        Selected::new(selected)?.into_result_set(accessor)
    }

    async fn execute(
        accessor: &Accessor<T, Self>, self_: Resource<DatabaseHandle>, sql: String,
        params: Vec<Option<Value>>,
    ) -> Result<u64> {
        let statement = statement(sql, params)?;
        let connection = DatabaseHandle::connection(accessor, &self_).await?;
        Ok(connection.execute(statement.sql, statement.params).await?)
    }

    async fn batch(
        accessor: &Accessor<T, Self>, self_: Resource<DatabaseHandle>,
        statements: Vec<wit::Statement>,
    ) -> Result<Vec<u64>> {
        let statements = statements
            .into_iter()
            .zip(1..)
            .map(|(given, n)| {
                statement(given.sql, given.params).map_err(|err| match err {
                    Error::Invalid(reason) => {
                        Error::Invalid(format!("statement {n} of the batch: {reason}"))
                    }
                    err => err,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if statements.is_empty() {
            return Ok(Vec::new());
        }

        let connection = DatabaseHandle::connection(accessor, &self_).await?;
        Ok(connection.batch(statements).await?)
    }

    fn drop(mut host: Access<'_, T, Self>, rep: Resource<DatabaseHandle>) -> wasmtime::Result<()> {
        host.get().table.delete(rep)?;
        Ok(())
    }
}

// The statement a backend runs, refused when it controls a transaction or
// binds a temporal value outside its case's format.
pub fn statement(sql: String, params: Vec<Option<Value>>) -> Result<Statement> {
    check(&sql)?;
    Ok(Statement {
        sql,
        params: value::bind(params)?,
    })
}

// Reads past leading whitespace, semicolons, and comments, taking block
// comments as nested, the way Postgres does. It keeps the contract, and is no
// security boundary.
fn check(sql: &str) -> Result<()> {
    let (first, rest) = word(sql);
    let (second, _) = word(rest);
    let refused = TRANSACTION_CONTROL.iter().any(|keyword| first.eq_ignore_ascii_case(keyword))
        || (first.eq_ignore_ascii_case("PREPARE") && second.eq_ignore_ascii_case("TRANSACTION"));
    if refused {
        return Err(Error::Invalid(format!(
            "`{}` starts or ends a transaction, which only `transactions` may do",
            first.to_ascii_uppercase()
        )));
    }
    Ok(())
}

// The next word and the text after it, or an empty word when the next token
// is not one.
fn word(sql: &str) -> (&str, &str) {
    let mut rest = sql;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ';');
        if let Some(comment) = rest.strip_prefix("--") {
            rest = comment.split_once('\n').map_or("", |(_, after)| after);
        } else if rest.starts_with("/*") {
            rest = past_block_comment(rest);
        } else {
            let end = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                .unwrap_or(rest.len());
            return rest.split_at(end);
        }
    }
}

// An unclosed comment runs to the end of the text.
fn past_block_comment(sql: &str) -> &str {
    let bytes = sql.as_bytes();
    let mut depth = 0_usize;
    let mut at = 0;
    while at + 1 < bytes.len() {
        match &bytes[at..at + 2] {
            b"/*" => {
                depth += 1;
                at += 2;
            }
            b"*/" => {
                depth -= 1;
                at += 2;
                if depth == 0 {
                    return &sql[at..];
                }
            }
            _ => at += 1,
        }
    }
    ""
}

// A query's columns and rows in their WIT form, every row in hand.
pub struct Selected {
    columns: Vec<String>,
    rows: Vec<types::Row>,
}

impl Selected {
    // A row that does not hold one value for each column is the backend's
    // fault, so it is `other`.
    pub fn new(selected: ResultSet) -> Result<Self> {
        let ResultSet { columns, rows } = selected;
        let rows = rows
            .into_iter()
            .zip(1..)
            .map(|(row, n)| {
                if row.fields.len() != columns.len() {
                    return Err(Error::Other(format!(
                        "row {n} holds {} values for {} columns",
                        row.fields.len(),
                        columns.len()
                    )));
                }
                Ok(row.fields.into_iter().map(|field| value::read(field.value)).collect())
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { columns, rows })
    }

    // `finished` resolves at once, since every row is already in hand.
    pub fn into_result_set<T>(self, accessor: &Accessor<T, WasiSql>) -> Result<types::ResultSet> {
        let Self { columns, rows } = self;
        accessor.with(|mut store| {
            let resolved = std::future::ready(Ok::<_, wasmtime::Error>(Ok(())));
            let mut finished = FutureReader::new(&mut store, resolved)?;
            let rows = match StreamReader::new(&mut store, rows) {
                Ok(rows) => rows,
                Err(error) => {
                    finished.close(&mut store)?;
                    return Err(error.into());
                }
            };
            Ok(types::ResultSet {
                columns,
                rows,
                finished,
            })
        })
    }
}

impl Host for HostView<'_, dyn SqlClient> {}

impl HostDatabase for HostView<'_, dyn SqlClient> {}
