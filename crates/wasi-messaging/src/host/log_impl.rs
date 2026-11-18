use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::host::generated::omnia::messaging::log::{
    Conflict, Host, HostWithStore, Precondition, Start,
};
use crate::host::generated::omnia::messaging::types::{self, Delivery, Pattern, Position, Topic};
use crate::host::resource::ClientProxy;
use crate::host::{Error, MessagingClient, Result, WasiMessaging};

impl<T> HostWithStore<T> for WasiMessaging {
    fn append(
        _: &Accessor<T, Self>, _: Resource<ClientProxy>, _: Topic, _: types::Message,
        _: Option<Precondition>,
    ) -> impl Future<Output = Result<std::result::Result<Position, Conflict>>> + Send {
        std::future::ready(Err(unsupported()))
    }

    fn read(
        _: &Accessor<T, Self>, _: Resource<ClientProxy>, _: Pattern, _: Start, _: Option<u32>,
    ) -> impl Future<Output = Result<Vec<Delivery>>> + Send {
        std::future::ready(Err(unsupported()))
    }
}

fn unsupported() -> Error {
    Error::Unsupported("this backend serves no log".to_owned())
}

impl Host for HostView<'_, dyn MessagingClient> {}
