//! `wasi:jsondb` `types` interface — `filter` resource constructors.

use wasmtime::component::{Access, Resource};

use crate::host::generated::wasi::jsondb::types as wit_types;
use crate::host::generated::wasi::jsondb::types::{
    ComparisonOp, Host as TypesHost, HostFilter, HostFilterWithStore, ScalarValue,
};
use crate::host::resource::{FilterProxy, FilterTree};
use crate::host::{JsonDbError, WasiJsonDb, WasiJsonDbCtxView};

const MAX_FILTER_DEPTH: usize = 5;
const MAX_IN_LIST_SIZE: usize = 100;

impl HostFilterWithStore for WasiJsonDb {
    fn compare<T>(
        mut host: Access<'_, T, Self>, field: String, op: ComparisonOp, value: ScalarValue,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = FilterTree::Compare { field, op, value };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn in_list<T>(
        mut host: Access<'_, T, Self>, field: String, values: Vec<ScalarValue>,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        wasmtime::ensure!(
            values.len() <= MAX_IN_LIST_SIZE,
            "in-list exceeds maximum of {MAX_IN_LIST_SIZE} values (got {})",
            values.len()
        );
        let tree = FilterTree::InList { field, values };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn not_in_list<T>(
        mut host: Access<'_, T, Self>, field: String, values: Vec<ScalarValue>,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        wasmtime::ensure!(
            values.len() <= MAX_IN_LIST_SIZE,
            "not-in-list exceeds maximum of {MAX_IN_LIST_SIZE} values (got {})",
            values.len()
        );
        let tree = FilterTree::NotInList { field, values };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn is_null<T>(
        mut host: Access<'_, T, Self>, field: String,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = FilterTree::IsNull(field);
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn is_not_null<T>(
        mut host: Access<'_, T, Self>, field: String,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = FilterTree::IsNotNull(field);
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn contains<T>(
        mut host: Access<'_, T, Self>, field: String, pattern: String,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = FilterTree::Contains { field, pattern };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn starts_with<T>(
        mut host: Access<'_, T, Self>, field: String, pattern: String,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = FilterTree::StartsWith { field, pattern };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn ends_with<T>(
        mut host: Access<'_, T, Self>, field: String, pattern: String,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let tree = FilterTree::EndsWith { field, pattern };
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn and<T>(
        mut host: Access<'_, T, Self>, filters: Vec<Resource<FilterProxy>>,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        wasmtime::ensure!(!filters.is_empty(), "filter.and requires at least one child");
        let mut children = Vec::with_capacity(filters.len());
        for r in filters {
            let fp = host.get().table.delete(r)?;
            children.push(fp.0);
        }
        let tree = FilterTree::And(children);
        check_depth(&tree)?;
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn or<T>(
        mut host: Access<'_, T, Self>, filters: Vec<Resource<FilterProxy>>,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        wasmtime::ensure!(!filters.is_empty(), "filter.or requires at least one child");
        let mut children = Vec::with_capacity(filters.len());
        for r in filters {
            let fp = host.get().table.delete(r)?;
            children.push(fp.0);
        }
        let tree = FilterTree::Or(children);
        check_depth(&tree)?;
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn not<T>(
        mut host: Access<'_, T, Self>, inner: Resource<FilterProxy>,
    ) -> wasmtime::Result<Resource<FilterProxy>> {
        let inner_tree = host.get().table.delete(inner)?;
        let tree = FilterTree::Not(Box::new(inner_tree.0));
        check_depth(&tree)?;
        Ok(host.get().table.push(FilterProxy(tree))?)
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<FilterProxy>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl HostFilter for WasiJsonDbCtxView<'_> {}

impl TypesHost for WasiJsonDbCtxView<'_> {
    fn convert_error(&mut self, err: JsonDbError) -> wasmtime::Result<wit_types::Error> {
        Ok(match err {
            JsonDbError::NoSuchStore => wit_types::Error::NoSuchStore,
            JsonDbError::AccessDenied => wit_types::Error::AccessDenied,
            JsonDbError::Other(s) => wit_types::Error::Other(s),
        })
    }
}

fn check_depth(tree: &FilterTree) -> wasmtime::Result<()> {
    let depth = tree.depth();
    wasmtime::ensure!(
        depth <= MAX_FILTER_DEPTH,
        "filter tree depth {depth} exceeds maximum of {MAX_FILTER_DEPTH}"
    );
    Ok(())
}
