//! The [`Endpoint`] seam a backend implements, and the [`Event`] that crosses
//! it.

use std::fmt::Debug;
use std::pin::Pin;

use futures::{FutureExt as _, Stream, future};
use omnia_core::FutureResult;

use crate::host::Error;

/// Stream of events.
pub type Events = Pin<Box<dyn Stream<Item = Event> + Send>>;

/// Stateful handle to a WebSocket endpoint.
///
/// A backend implements [`events`](Self::events) and [`send`](Self::send),
/// and overrides each other method it serves. A call it does not serve fails
/// with [`Error::Unsupported`].
pub trait Endpoint: Debug + Send + Sync + 'static {
    /// Subscribe to incoming events from WebSocket clients.
    fn events(&self) -> FutureResult<Events>;

    /// Send an event to each open connection named in `sockets`, or to every
    /// open connection when `None`.
    ///
    /// A named connection that is not open is skipped.
    fn send(&self, event: Event, sockets: Option<Vec<String>>) -> FutureResult<()>;

    /// Send an event to every open connection except those named in `except`.
    ///
    /// An id in `except` that is not open is skipped. A backend that does not
    /// override it sends an empty `except` through [`send`](Self::send), and
    /// fails with [`Error::Unsupported`] for any other.
    fn broadcast(&self, event: Event, except: Vec<String>) -> FutureResult<()> {
        if except.is_empty() {
            return self.send(event, None);
        }
        unsupported("exclude connections from a broadcast")
    }

    /// Close the connection `connection`, sending `frame` to its peer.
    ///
    /// `None` sends a close frame with no status code. Closing a connection
    /// that is not open succeeds.
    fn close(&self, connection: String, frame: Option<CloseFrame>) -> FutureResult<()> {
        let _ = (connection, frame);
        unsupported("close a connection")
    }

    /// Whether the connection `connection` is open.
    fn exists(&self, connection: String) -> FutureResult<bool> {
        let _ = connection;
        unsupported("tell whether a connection is open")
    }
}

fn unsupported<T: Send + 'static>(what: &str) -> FutureResult<T> {
    let error = Error::Unsupported(format!("this backend cannot {what}"));
    future::ready(Err(error.into())).boxed()
}

/// Proxy for a WebSocket server endpoint.
pub type ClientProxy = omnia_core::Proxy<dyn Endpoint>;

/// A WebSocket event crossing the boundary.
///
/// The host owns event state; backends translate to and from their wire
/// representation at the [`Endpoint`] seam. Non-exhaustive so a new field is
/// not a breaking change: construct via [`Event::new`] and set fields
/// directly.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Event {
    /// The id of the connection the event came from.
    ///
    /// The backend mints it, and never gives a closed connection's id to
    /// another. The host refuses an event without one.
    pub socket_addr: Option<String>,
    /// The event data.
    pub data: Vec<u8>,
    /// The frame type `data` travels in.
    pub frame: Frame,
    /// The route key used to select a guest, when the event carries one.
    ///
    /// `None` fans the event into the trigger's catch-all guest (the sole
    /// websocket exporter), preserving single-guest behaviour.
    pub route: Option<String>,
}

impl Event {
    /// Create a binary event with the given payload.
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            ..Self::default()
        }
    }
}

/// The frame type an [`Event`]'s data travels in, as RFC 6455 defines it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Frame {
    /// Bytes, opaque to the host.
    #[default]
    Binary,
    /// Text, whose data is UTF-8.
    Text,
}

/// The status a close frame carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseFrame {
    /// The status code, as RFC 6455 section 7.4 defines it.
    pub code: u16,
    /// Why the connection closed, at most 123 bytes of UTF-8.
    pub reason: String,
}
