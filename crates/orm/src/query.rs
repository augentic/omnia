use anyhow::Result;
use sea_query::backend::{
    EscapeBuilder, OperLeftAssocDecider, PrecedenceDecider, QuotedBuilder, TableRefBuilder,
};
use sea_query::prepare::SqlWriter;
use sea_query::{BinOper, Oper, QueryStatementWriter, Quote, SimpleExpr, SubQueryStatement, Value};

use crate::DataType;
use crate::entity::values_to_wasi_datatypes;

/// A compiled SQL query ready for execution.
pub struct Query {
    /// The SQL query string.
    pub sql: String,
    /// The bound parameter values.
    pub params: Vec<DataType>,
}

/// Parameterised query builder targeting Postgres/SQLite (`$1, $2, ...` placeholders).
pub struct QueryBuilder;

impl QuotedBuilder for QueryBuilder {
    fn quote(&self) -> Quote {
        Quote::new(b'"')
    }
}

impl EscapeBuilder for QueryBuilder {}

impl TableRefBuilder for QueryBuilder {}

impl OperLeftAssocDecider for QueryBuilder {
    fn well_known_left_associative(&self, op: &BinOper) -> bool {
        matches!(
            op,
            BinOper::And | BinOper::Or | BinOper::Add | BinOper::Sub | BinOper::Mul | BinOper::Mod
        )
    }
}

impl PrecedenceDecider for QueryBuilder {
    fn inner_expr_well_known_greater_precedence(
        &self, _inner: &SimpleExpr, _outer_oper: &Oper,
    ) -> bool {
        false
    }
}

impl sea_query::backend::QueryBuilder for QueryBuilder {
    fn prepare_query_statement(&self, query: &SubQueryStatement, sql: &mut dyn SqlWriter) {
        match query {
            SubQueryStatement::SelectStatement(s) => self.prepare_select_statement(s, sql),
            SubQueryStatement::InsertStatement(s) => self.prepare_insert_statement(s, sql),
            SubQueryStatement::UpdateStatement(s) => self.prepare_update_statement(s, sql),
            SubQueryStatement::DeleteStatement(s) => self.prepare_delete_statement(s, sql),
            SubQueryStatement::WithStatement(s) => self.prepare_with_query(s, sql),
        }
    }

    fn prepare_value(&self, value: &Value, sql: &mut dyn SqlWriter) {
        sql.push_param(value.clone(), self);
    }

    fn placeholder(&self) -> (&str, bool) {
        ("$", true)
    }
}

/// Builds a [`Query`] from any `SeaQuery` statement, providing an escape hatch for guests
/// who need to construct queries directly with `SeaQuery` rather than through the ORM builders.
///
/// # Errors
///
/// Returns an error if any query parameter values cannot be converted to WASI data types.
pub fn build_query(statement: &impl QueryStatementWriter) -> Result<Query> {
    let (sql, values) = statement.build(QueryBuilder);
    let params = values_to_wasi_datatypes(values)?;

    tracing::debug!(
        sql = %sql,
        param_count = params.len(),
        "build_query generated SQL"
    );

    Ok(Query { sql, params })
}
