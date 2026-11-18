//! The `omnia:messaging/consumer` host bindings, which answer every call with
//! `unsupported`.

use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::omnia::messaging::consumer::{
    Host, HostLease, HostLeaseWithStore, HostWithStore, Lease,
};
use crate::host::generated::omnia::messaging::types::Delivery;
use crate::host::resource::ClientProxy;
use crate::host::{Error, MessagingClient, Result, WasiMessaging};

impl<T> HostWithStore<T> for WasiMessaging {
    fn receive(
        _: &Accessor<T, Self>, _: Resource<ClientProxy>, _: String, _: u32, _: u32,
    ) -> impl Future<Output = Result<Vec<Resource<Lease>>>> + Send {
        std::future::ready(Err(unsupported()))
    }
}

fn unsupported() -> Error {
    Error::Unsupported("this backend serves no consumer".to_owned())
}

// no `lease` handle exists while `receive` refuses every call
impl<T> HostLeaseWithStore<T> for WasiMessaging {
    fn delivery(_: Access<'_, T, Self>, _: Resource<Lease>) -> wasmtime::Result<Delivery> {
        Err(unsupported().into())
    }

    fn accept(
        _: &Accessor<T, Self>, _: Resource<Lease>,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(Err(unsupported()))
    }

    fn release(
        _: &Accessor<T, Self>, _: Resource<Lease>, _: Option<u32>,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(Err(unsupported()))
    }

    fn reject(
        _: &Accessor<T, Self>, _: Resource<Lease>,
    ) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(Err(unsupported()))
    }

    fn renew(_: &Accessor<T, Self>, _: Resource<Lease>) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(Err(unsupported()))
    }

    fn drop(mut host: Access<'_, T, Self>, rep: Resource<Lease>) -> wasmtime::Result<()> {
        match host.get().table.delete(rep)? {}
    }
}

impl Host for HostView<'_, dyn MessagingClient> {}

impl HostLease for HostView<'_, dyn MessagingClient> {}
