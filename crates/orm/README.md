# Omnia ORM

A lightweight object-relational mapper based on [`sea-query`](https://crates.io/crates/sea-query) but completely backend agnostic. This crate is intended as a helper for guests using `omnia-wasi-sql` to assist in query building and mapping return values to business structures.

This crate uses types from `omnia-wasi-sql` and re-exports `Row` and `DataType` for convenience.

It is intended that this crate is used in guest components only.
