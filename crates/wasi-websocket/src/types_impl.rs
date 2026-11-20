//! The `omnia:websocket/types` host bindings, and the conversions between its
//! records and the seam's [`Event`] and [`CloseFrame`].

use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, Resource};

use crate::generated::omnia::websocket::types::{self, Host, HostClient, HostClientWithStore};
use crate::resource::{ClientProxy, CloseFrame, Event, Frame};
use crate::{Error, Result, WasiWebSocket, WebSocketClient};

// the one service the endpoint seam reaches
const DEFAULT: &str = "default";

// RFC 6455 caps a control frame's payload at 125 bytes, two of them the code
const MAX_REASON_LEN: usize = 123;

pub fn get_client<T>(
    accessor: &Accessor<T, WasiWebSocket>, self_: &Resource<ClientProxy>,
) -> Result<ClientProxy> {
    Ok(omnia_core::get_cloned(accessor, self_)?)
}

impl<T> HostClientWithStore<T> for WasiWebSocket {
    async fn connect(accessor: &Accessor<T, Self>, name: String) -> Result<Resource<ClientProxy>> {
        if name != DEFAULT {
            return Err(Error::NotFound(format!(
                "the deployment declares no websocket service `{name}`"
            )));
        }
        let client = accessor.with(|mut store| store.get().client.endpoint()).await?;
        let proxy = omnia_core::Proxy(client);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<ClientProxy>) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for HostView<'_, dyn WebSocketClient> {}

impl HostClient for HostView<'_, dyn WebSocketClient> {}

impl From<types::Message> for Event {
    fn from(message: types::Message) -> Self {
        let (frame, data) = match message {
            types::Message::Text(text) => (Frame::Text, text.into_bytes()),
            types::Message::Binary(data) => (Frame::Binary, data),
        };
        Self {
            frame,
            ..Self::new(data)
        }
    }
}

impl TryFrom<Event> for types::Message {
    type Error = Error;

    fn try_from(event: Event) -> Result<Self> {
        match event.frame {
            Frame::Binary => Ok(Self::Binary(event.data)),
            Frame::Text => String::from_utf8(event.data)
                .map(Self::Text)
                .map_err(|err| Error::Invalid(format!("a text event is not UTF-8: {err}"))),
        }
    }
}

// the seam carries no backend metadata yet, and only the trigger sets a
// deadline
impl TryFrom<Event> for types::Event {
    type Error = Error;

    fn try_from(mut event: Event) -> Result<Self> {
        let Some(connection) = event.socket_addr.take() else {
            return Err(Error::Invalid("the event names no connection".to_owned()));
        };
        let route = event.route.take();
        Ok(Self {
            connection,
            route,
            message: types::Message::try_from(event)?,
            metadata: Vec::new(),
            deadline: None,
        })
    }
}

// RFC 6455 section 7.4 reserves 1004, and forbids an endpoint to send 1005,
// 1006 or 1015
impl TryFrom<types::CloseFrame> for CloseFrame {
    type Error = Error;

    fn try_from(frame: types::CloseFrame) -> Result<Self> {
        if frame.reason.len() > MAX_REASON_LEN {
            return Err(Error::Invalid(format!(
                "a close reason is at most {MAX_REASON_LEN} bytes of UTF-8"
            )));
        }
        if !(1000..=4999).contains(&frame.code) || matches!(frame.code, 1004..=1006 | 1015) {
            return Err(Error::Invalid(format!("no endpoint may send close code {}", frame.code)));
        }
        Ok(Self {
            code: frame.code,
            reason: frame.reason,
        })
    }
}
