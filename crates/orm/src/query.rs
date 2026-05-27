use anyhow::Result;
use sea_query::backend::{
    EscapeBuilder, OperLeftAssocDecider, PrecedenceDecider, QuotedBuilder, TableRefBuilder,
};
use sea_query::prepare::SqlWriter;
use sea_query::{
    BinOper, Oper, QueryStatementBuilder, Quote, SimpleExpr, SubQueryStatement, Value,
};

use crate::DataType;
use crate::entity::values_to_wasi_datatypes;

pub struct Query {
    pub sql: String,
    pub params: Vec<DataType>,
}

/// Finalises a `SeaQuery` statement into a [`Query`]: renders the SQL, converts the bound
/// values to WASI [`DataType`]s, and emits a uniform `tracing::debug!` event.
pub fn finish<S: QueryStatementBuilder>(
    stmt: &S, table: &'static str, kind: &'static str,
) -> Result<Query> {
    let (sql, values) = stmt.build_any(&QueryBuilder);
    let params = values_to_wasi_datatypes(values)?;

    tracing::debug!(
        table,
        kind,
        sql = %sql,
        param_count = params.len(),
        "ORM query built",
    );

    Ok(Query { sql, params })
}

/// Backend-agnostic `SeaQuery` query builder configured for Postgres/SQLite dialects:
/// double-quoted identifiers and numbered placeholders (`$1`, `$2`, ...).
#[derive(Default)]
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
        // Copied from sea-query 0.32.7 backend/query_builder.rs `common_well_known_left_associative`
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
        // Conservative approach that forces parentheses
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
