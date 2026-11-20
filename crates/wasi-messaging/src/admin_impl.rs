//! The `omnia:messaging/admin` host bindings, which answer every call with
//! `unsupported`.

use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::generated::omnia::messaging::admin::{Host, HostWithStore, Names};
use crate::generated::omnia::messaging::types::Pattern;
use crate::resource::ClientProxy;
use crate::{Error, MessagingClient, Result, WasiMessaging};

impl<T> HostWithStore<T> for WasiMessaging {
    fn list_topics(
        _: &Accessor<T, Self>, _: Resource<ClientProxy>, _: Pattern, _: Option<String>,
        _: Option<u32>,
    ) -> impl Future<Output = Result<Names>> + Send {
        std::future::ready(Err(unsupported()))
    }

    fn purge_topics(
        _: &Accessor<T, Self>, _: Resource<ClientProxy>, _: Pattern,
    ) -> impl Future<Output = Result<u64>> + Send {
        std::future::ready(Err(unsupported()))
    }
}

fn unsupported() -> Error {
    Error::Unsupported("this backend serves no topic administration".to_owned())
}

impl Host for HostView<'_, dyn MessagingClient> {}
