use omnia_core::HostView;
use opentelemetry_sdk::Resource;

use crate::host::OtelClient;
use crate::host::generated::omnia::otel::{resource, types};

impl resource::Host for HostView<'_, dyn OtelClient> {
    fn resource(&mut self) -> wasmtime::Result<types::Resource> {
        let Some(resource) = omnia_core::telemetry::resource() else {
            ::tracing::warn!("otel resource not initialized");
            let empty = &Resource::builder().build();
            return Ok(empty.into());
        };
        Ok(resource.into())
    }
}
