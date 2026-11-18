//! The `omnia:messaging/types` host bindings, and the conversions between its
//! records and the seam's [`Message`].

use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::omnia::messaging::types::{
    self, Delivery, Host, HostClient, HostClientWithStore,
};
use crate::host::resource::{ClientProxy, Message, Metadata};
use crate::host::{Error, MessagingClient, Result, WasiMessaging};

// the one broker the endpoint seam reaches
const DEFAULT: &str = "default";

// the metadata entries the typed fields travel as across the endpoint seam
const CONTENT_TYPE: &str = "content-type";
const KEY: &str = "key";

pub fn get_client<T>(
    accessor: &Accessor<T, WasiMessaging>, self_: &Resource<ClientProxy>,
) -> Result<ClientProxy> {
    Ok(omnia_core::get_cloned(accessor, self_)?)
}

impl<T> HostClientWithStore<T> for WasiMessaging {
    async fn connect(accessor: &Accessor<T, Self>, name: String) -> Result<Resource<ClientProxy>> {
        if name != DEFAULT {
            return Err(Error::NotFound(format!("the deployment declares no broker `{name}`")));
        }
        let client = accessor.with(|mut store| store.get().client.endpoint()).await?;
        let proxy = omnia_core::Proxy(client);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<ClientProxy>) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for HostView<'_, dyn MessagingClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

impl HostClient for HostView<'_, dyn MessagingClient> {}

// the seam carries `content-type` and `key` as the metadata entries of those
// names, each replacing an entry of that name in any case, and carries no id
impl TryFrom<types::Message> for Message {
    type Error = Error;

    fn try_from(message: types::Message) -> Result<Self> {
        if message.id.is_some() {
            return Err(Error::Unsupported("no backend carries a message id".to_owned()));
        }

        let mut metadata = Metadata::from(message.metadata);
        for (name, value) in [(CONTENT_TYPE, message.content_type), (KEY, message.key)] {
            if let Some(value) = value {
                metadata.retain(|entry, _| !entry.eq_ignore_ascii_case(name));
                metadata.insert(name.to_owned(), value);
            }
        }

        let mut outgoing = Self::new(message.data);
        outgoing.metadata = (!metadata.is_empty()).then_some(metadata);
        Ok(outgoing)
    }
}

impl From<Message> for types::Message {
    fn from(message: Message) -> Self {
        let mut metadata = message.metadata.unwrap_or_default();
        let content_type = take(&mut metadata, CONTENT_TYPE);
        let key = take(&mut metadata, KEY);
        Self {
            data: message.payload,
            content_type,
            id: None,
            key,
            metadata: metadata.into(),
        }
    }
}

// no backend reports a position, an attempt count, or a publish time yet
impl From<Message> for Delivery {
    fn from(mut message: Message) -> Self {
        let topic = std::mem::take(&mut message.topic);
        let reply_to = message.reply.take().map(|reply| reply.topic);
        Self {
            topic,
            message: message.into(),
            reply_to,
            position: None,
            attempt: None,
            published_at: None,
        }
    }
}

// removes every entry named `name`, in any case, answering one of their values
fn take(metadata: &mut Metadata, name: &str) -> Option<String> {
    let mut value = None;
    metadata.retain(|entry, entry_value| {
        let matched = entry.eq_ignore_ascii_case(name);
        if matched {
            value = Some(std::mem::take(entry_value));
        }
        !matched
    });
    value
}
