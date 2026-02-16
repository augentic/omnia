use sea_query::backend::{
    EscapeBuilder, OperLeftAssocDecider, PrecedenceDecider, QuotedBuilder, TableRefBuilder,
};
use sea_query::prepare::SqlWriter;
use sea_query::{BinOper, Oper, Quote, SimpleExpr, SubQueryStatement, Value};

use crate::DataType;

pub struct Query {
    pub sql: String,
    pub params: Vec<DataType>,
}

pub struct QueryBuilder {
    pub quote: Quote,
    pub placeholder: &'static str, // "?" or "$"
    pub numbered: bool,            // false for "?", true for "$1, $2, ..."
}

impl Default for QueryBuilder {
    // should work for `Postgres` and `Sqlite`
    fn default() -> Self {
        Self {
            quote: Quote::new(b'"'),
            placeholder: "$",
            numbered: true,
        }
    }
}

impl QuotedBuilder for QueryBuilder {
    fn quote(&self) -> Quote {
        self.quote
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
        (self.placeholder, self.numbered)
    }
}
