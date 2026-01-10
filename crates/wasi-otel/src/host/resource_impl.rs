use anyhow::Result;
use opentelemetry_sdk::Resource;

use crate::host::WasiOtelCtxView;
use crate::host::generated::wasi::otel::{resource, types};

impl resource::Host for WasiOtelCtxView<'_> {
    fn resource(&mut self) -> Result<types::Resource> {
        let Some(resource) = yetti_otel::init::resource() else {
            ::tracing::warn!("otel resource not initialized");
            let empty = &Resource::builder().build();
            return Ok(empty.into());
        };
        Ok(resource.into())
    }
}
