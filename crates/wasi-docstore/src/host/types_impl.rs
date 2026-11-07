//! `omnia:docstore` `types` interface — `filter` resource constructors.

use wasmtime::component::{Access, Resource};

use crate::host::generated::omnia::docstore::types::{
    ComparisonOp as WitComparisonOp, Error, Host as TypesHost, HostFilter, HostFilterWithStore,
    ScalarValue as WitScalarValue,
};
use crate::host::resource::FilterProxy;
use crate::host::{ComparisonOp, Filter, ScalarValue, WasiDocStore, WasiDocStoreView};

const MAX_FILTER_DEPTH: usize = 5;
const MAX_IN_LIST_SIZE: usize = 100;

fn combine<T>(
    mut host: Access<'_, T, WasiDocStore>, filters: Vec<Resource<FilterProxy>>, verb: &str,
    node: fn(Vec<Filter>) -> Filter,
) -> wasmtime::Result<Resource<FilterProxy>> {
    wasmtime::ensure!(!filters.is_empty(), "{verb} requires at least one child");
    let mut children = Vec::with_capacity(filters.len());
    for r in filters {
        let fp = host.get().table.delete(r)?;
        children.push(fp.0);
    }
    let tree = node(children);
    check_depth(&tree)?;
    Ok(host.get().table.push(FilterProxy(tree))?)
}

fn check_depth(tree: &Filter) -> wasmtime::Result<()> {
    let depth = tree.depth();
    wasmtime::ensure!(
        depth <= MAX_FILTER_DEPTH,
        "filter tree depth {depth} exceeds maximum of {MAX_FILTER_DEPTH}"
    );
    Ok(())
}

const fn from_wit_op(op: WitComparisonOp) -> ComparisonOp {
    match op {
        WitComparisonOp::Eq => ComparisonOp::Eq,
        WitComparisonOp::Ne => ComparisonOp::Ne,
        WitComparisonOp::Gt => ComparisonOp::Gt,
        WitComparisonOp::Gte => ComparisonOp::Gte,
        WitComparisonOp::Lt => ComparisonOp::Lt,
        WitComparisonOp::Lte => ComparisonOp::Lte,
    }
}

fn from_wit_value(v: WitScalarValue) -> ScalarValue {
    match v {
        WitScalarValue::Null => ScalarValue::Null,
        WitScalarValue::Boolean(b) => ScalarValue::Bool(b),
        WitScalarValue::Int32(i) => ScalarValue::Int32(i),
        WitScalarValue::Int64(i) => ScalarValue::Int64(i),
        WitScalarValue::Float64(f) => ScalarValue::Float64(f),
        WitScalarValue::Str(s) => ScalarValue::Str(s),
        WitScalarValue::Binary(b) => ScalarValue::Binary(b),
        WitScalarValue::Timestamp(t) => ScalarValue::Timestamp(t),
    }
}

impl<T> HostFilterWithStore<T> for WasiDocStore {
    fn compare(
        mut host: Access<'_, T, Self>, field: String, op: WitComparisonOp, value: WitScalarValue,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = Filter::Compare {
            field,
            op: from_wit_op(op),
            value: from_wit_value(value),
        };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn in_list(
        mut host: Access<'_, T, Self>, field: String, values: Vec<WitScalarValue>,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        wasmtime::ensure!(
            values.len() <= MAX_IN_LIST_SIZE,
            "in-list exceeds maximum of {MAX_IN_LIST_SIZE} values (got {})",
            values.len()
        );
        let tree = Filter::InList {
            field,
            values: values.into_iter().map(from_wit_value).collect(),
        };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn not_in_list(
        mut host: Access<'_, T, Self>, field: String, values: Vec<WitScalarValue>,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        wasmtime::ensure!(
            values.len() <= MAX_IN_LIST_SIZE,
            "not-in-list exceeds maximum of {MAX_IN_LIST_SIZE} values (got {})",
            values.len()
        );
        let tree = Filter::NotInList {
            field,
            values: values.into_iter().map(from_wit_value).collect(),
        };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn is_null(
        mut host: Access<'_, T, Self>, field: String,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = Filter::IsNull(field);
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn is_not_null(
        mut host: Access<'_, T, Self>, field: String,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = Filter::IsNotNull(field);
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn contains(
        mut host: Access<'_, T, Self>, field: String, pattern: String,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = Filter::Contains { field, pattern };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn starts_with(
        mut host: Access<'_, T, Self>, field: String, pattern: String,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = Filter::StartsWith { field, pattern };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn ends_with(
        mut host: Access<'_, T, Self>, field: String, pattern: String,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = Filter::EndsWith { field, pattern };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn and(
        host: Access<'_, T, Self>, filters: Vec<Resource<FilterProxy>>,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        combine(host, filters, "filter.and", Filter::And)
    }

    fn or(
        host: Access<'_, T, Self>, filters: Vec<Resource<FilterProxy>>,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        combine(host, filters, "filter.or", Filter::Or)
    }

    fn not(
        mut host: Access<'_, T, Self>, inner: Resource<FilterProxy>,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let inner_tree = host.get().table.delete(inner)?;
        let tree = Filter::Not(Box::new(inner_tree.0));
        check_depth(&tree)?;
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<FilterProxy>) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl HostFilter for WasiDocStoreView<'_> {}

impl TypesHost for WasiDocStoreView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}
