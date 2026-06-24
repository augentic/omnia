use wasmtime::component::{Accessor, Resource};

use crate::host::generated::wasi::messaging::producer::{Host, HostWithStore};
use crate::host::generated::wasi::messaging::types::Topic;
use crate::host::resource::{ClientProxy, MessageProxy};
use crate::host::types_impl::{get_client, get_message};
use crate::host::{Result, WasiMessaging, WasiMessagingCtxView};

impl<T> HostWithStore<T> for WasiMessaging {
    async fn send(
        accessor: &Accessor<T, Self>, c: Resource<ClientProxy>, topic: Topic,
        message: Resource<MessageProxy>,
    ) -> Result<()> {
        let client = get_client(accessor, &c)?;
        let msg = get_message(accessor, &message)?;
        client.send(topic, msg).await?;

        Ok(())
    }
}

impl Host for WasiMessagingCtxView<'_> {}
