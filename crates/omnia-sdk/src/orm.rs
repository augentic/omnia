//! # ORM
//!
//! A lightweight object-relational mapper based on [`sea-query`](https://crates.io/crates/sea-query)
//! but completely backend agnostic. This module is intended as a helper for guests using
//! `omnia-wasi-sql` to assist in query building and mapping return values to business structures.
//!
//! It re-exports `Row` and `DataType` from `omnia-wasi-sql` for convenience.
#![forbid(unsafe_code)]

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
pub use filter::{CmpOp, ColRef, Filter};
pub use insert::InsertBuilder;
pub use join::{Join, JoinKind};
pub use omnia_wasi_sql::{DataType, Field, Row};
pub use select::SelectBuilder;
pub use update::UpdateBuilder;

#[doc(hidden)]
pub mod __private {
    pub use sea_query::Value;
}

/// Declares an ORM entity with automatic `Entity` trait implementation.
///
/// # Examples
///
/// ```ignore
/// use omnia_sdk::orm::entity;
///
/// entity! {
///     table = "posts",
///     pub struct Post {
///         pub id: i32,
///         pub title: String,
///     }
/// }
/// ```
#[macro_export]
macro_rules! entity {
    // Full form: columns + joins + struct (single code-generation arm)
    (
        table = $table:literal,
        columns = [$( ($col_table:literal, $col_name:literal, $col_field:literal) ),* $(,)?],
        joins = [$($join:expr),* $(,)?],
        $(#[$meta:meta])*
        pub struct $struct_name:ident {
            $(
                $(#[$field_meta:meta])*
                pub $field_name:ident : $field_type:ty
            ),* $(,)?
        }
    ) => {
        #[allow(missing_docs)]
        $(#[$meta])*
        pub struct $struct_name {
            $(
                $(#[$field_meta])*
                pub $field_name : $field_type
            ),*
        }

        impl $crate::orm::Entity for $struct_name {
            const TABLE: &'static str = $table;

            fn projection() -> &'static [&'static str] {
                &[ $( stringify!($field_name) ),* ]
            }

            fn joins() -> Vec<$crate::orm::Join> {
                vec![$($join),*]
            }

            fn column_specs() -> Vec<(&'static str, &'static str, &'static str)> {
                vec![$( ($col_field, $col_table, $col_name) ),*]
            }

            fn from_row(row: &$crate::orm::Row) -> anyhow::Result<Self> {
                Ok(Self {
                    $(
                        $field_name: <$field_type as $crate::orm::FetchValue>::fetch(row, stringify!($field_name))?,
                    )*
                })
            }
        }

        impl $crate::orm::EntityValues for $struct_name {
            fn __to_values(&self) -> Vec<(&'static str, $crate::orm::__private::Value)> {
                vec![
                    $(
                        (stringify!($field_name), self.$field_name.clone().into()),
                    )*
                ]
            }
        }
    };

    // Joins only → forward with empty columns
    (
        table = $table:literal,
        joins = [$($join:expr),* $(,)?],
        $($rest:tt)*
    ) => {
        $crate::entity! {
            table = $table,
            columns = [],
            joins = [$($join),*],
            $($rest)*
        }
    };

    // Bare table → forward with empty columns and joins
    (
        table = $table:literal,
        $($rest:tt)*
    ) => {
        $crate::entity! {
            table = $table,
            columns = [],
            joins = [],
            $($rest)*
        }
    };
}
