use omnia_core::HostView;
use wasmtime::component::Accessor;

use crate::host::generated::omnia::otel::logs::{Host, HostWithStore, ScopeLogs};
use crate::host::{Error, OtelClient, Result, WasiOtel};

impl<T> HostWithStore<T> for WasiOtel {
    fn export(_: &Accessor<T, Self>, _: Vec<ScopeLogs>) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(Err(Error::Unsupported("this backend exports no logs".to_owned())))
    }
}

impl Host for HostView<'_, dyn OtelClient> {}
