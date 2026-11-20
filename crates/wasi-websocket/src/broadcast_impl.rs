//! The `omnia:websocket/broadcast` host bindings.

use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::host::generated::omnia::websocket::broadcast::{Host, HostWithStore};
use crate::host::generated::omnia::websocket::types::{self, ConnectionId};
use crate::host::resource::{ClientProxy, Event};
use crate::host::types_impl::get_client;
use crate::host::{Result, WasiWebSocket, WebSocketClient};

impl<T> HostWithStore<T> for WasiWebSocket {
    async fn send(
        accessor: &Accessor<T, Self>, c: Resource<ClientProxy>, message: types::Message,
        except: Vec<ConnectionId>,
    ) -> Result<()> {
        let client = get_client(accessor, &c)?;
        client.broadcast(Event::from(message), except).await?;

        Ok(())
    }
}

impl Host for HostView<'_, dyn WebSocketClient> {}
