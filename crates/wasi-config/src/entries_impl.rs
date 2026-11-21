use omnia_core::HostView;
use wasmtime::component::Accessor;

use crate::generated::omnia::config::entries::{Host, HostWithStore, Page};
use crate::{ConfigClient, Error, Result, WasiConfig};

const PAGE_LIMIT: u32 = 1_000;

impl<T> HostWithStore<T> for WasiConfig {
    async fn list(
        accessor: &Accessor<T, Self>, prefix: String, after: Option<String>, limit: Option<u32>,
    ) -> Result<Page> {
        let limit = match limit {
            Some(0) => return Err(Error::Invalid("a listing limit must be at least 1".to_owned())),
            Some(limit) => limit.min(PAGE_LIMIT) as usize,
            None => PAGE_LIMIT as usize,
        };
        let entries = accessor.with(|mut store| store.get().client.entries()).await?;
        Ok(page(entries, &prefix, after.as_deref(), limit))
    }
}

// A backend lists every entry at once, so the host filters, orders, and pages.
fn page(
    mut entries: Vec<(String, String)>, prefix: &str, after: Option<&str>, limit: usize,
) -> Page {
    entries.retain(|(key, _)| {
        key.starts_with(prefix) && after.is_none_or(|after| key.as_str() > after)
    });
    entries.sort_unstable();
    let next = (entries.len() > limit).then(|| entries[limit - 1].0.clone());
    entries.truncate(limit);
    Page { entries, next }
}

impl Host for HostView<'_, dyn ConfigClient> {}
