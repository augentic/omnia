#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg(target_arch = "wasm32")]

mod delete;
mod entity;
mod filter;
mod insert;
mod join;
mod query;
mod select;
mod update;

pub use delete::DeleteBuilder;
pub use entity::{Entity, EntityValues, FetchValue};
pub use filter::Filter;
pub use insert::InsertBuilder;
pub use join::Join;
pub use omnia_wasi_sql::{DataType, Field, Row};
pub use query::{Query, QueryBuilder, build_query};
pub use sea_query::{JoinType, Order};
pub use select::SelectBuilder;
pub use update::UpdateBuilder;

#[doc(hidden)]
pub mod __private {
    pub use sea_query::Value;
}
