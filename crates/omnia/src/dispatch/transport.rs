//! # Link transport seam
//!
//! Host-mediated calls ride [wRPC](https://github.com/bytecodealliance/wrpc) on
//! every leg; what is pluggable is the wRPC *transport*, not the RPC framework.
//! [`LinkTransport`] is that seam: the dispatch path only ever asks it to open a
//! client connection to a target, so "desktop -> cloud" becomes a transport
//! swap rather than a code change.
//!
//! Today it has one implementation, [`InProcess`]: full wRPC encode/decode over
//! an in-memory [`tokio::io::duplex`] byte pipe, with no network. Unix-domain
//! sockets, NATS and QUIC would slot in behind the same trait.
//!
//! The serve side is the registry itself: each target guest that exports a
//! host-mediated interface runs a wRPC [`Server`] whose handlers instantiate the
//! guest *fresh per call*. The carrier mints a fresh connection to that server
//! per invocation — closing the single-use limitation of a bare
//! [`Oneshot`](wrpc_transport::frame::Oneshot).

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::{Arc, PoisonError, RwLock};

use anyhow::{Context as _, Result, ensure};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf, split};
use wasmtime::component::ResourceTable;
use wrpc_transport::frame::{Oneshot, Server};
use wrpc_wasmtime::{SharedResourceTable, WrpcCtx, WrpcCtxView};

use crate::registry::GuestId;

/// Default in-process pipe buffer size (64 kibibytes).
const DUPLEX_BUF: usize = 1 << 16;

/// The in-process wRPC server type: framed transport over a `tokio::io::duplex`
/// byte stream, one connection accepted per dispatched call.
pub type InProcServer = Server<(), ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

/// The in-process wRPC client handle: a single stream pair to one target's
/// server, used for exactly one invocation.
pub type InProcClient = Oneshot<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

/// The wRPC client handle type a guest store advertises to `wrpc-wasmtime`.
///
/// Re-exported for the `runtime!` macro's generated [`wrpc_wasmtime::WrpcView`]
/// implementation; equal to the in-process carrier's client.
pub type LinkClient = InProcClient;

/// A bound wRPC transport. The dispatch path talks only to this — never to a
/// concrete transport — so the same selector-driven dispatch runs co-located or
/// distributed.
///
/// Only [`InProcess`] implements it today; the trait is the seam a distributed
/// transport (UDS / NATS / QUIC) would extend.
pub trait LinkTransport: Send + Sync + 'static {
    /// The wRPC client handle this transport hands the dispatch path.
    type Client: wrpc_transport::Invoke<Context = ()>;

    /// Open a fresh client connection to `target` for a single invocation.
    ///
    /// # Errors
    ///
    /// Returns an error if `target` has no bound endpoint on this transport.
    fn connect(&self, target: &GuestId) -> Result<Self::Client>;
}

/// The co-located fast transport: every target's exports are served over a wRPC
/// [`Server`] reachable through an in-memory byte pipe.
///
/// The server map is `Arc`-shared behind interior mutability so a guest
/// registered after bootstrap gains an endpoint on every clone of the carrier
/// (serve-at-register). Reads additionally pass through the shared lifecycle
/// gate, so a lookup never observes a half-applied register or deregister;
/// mutations run with the lifecycle write guard already held by the caller
/// (the registry's transactional publish/remove).
#[derive(Clone)]
pub struct InProcess {
    servers: Arc<RwLock<HashMap<GuestId, Arc<InProcServer>>>>,
    // The dispatch handle's lifecycle gate (see `DispatchHandle::lifecycle`).
    // Lock order: lifecycle first, then `servers` — never the reverse.
    lifecycle: Arc<RwLock<()>>,
}

impl InProcess {
    /// Create an empty carrier sharing the dispatch handle's lifecycle gate;
    /// [`super::serve_links`] and dynamic registration populate it.
    #[must_use]
    pub(crate) fn new(lifecycle: Arc<RwLock<()>>) -> Self {
        Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
            lifecycle,
        }
    }

    /// Returns the wRPC server serving `target`'s host-mediated exports, if any.
    #[must_use]
    pub fn server(&self, target: &GuestId) -> Option<Arc<InProcServer>> {
        let _lifecycle = self.lifecycle.read().unwrap_or_else(PoisonError::into_inner);
        self.servers.read().unwrap_or_else(PoisonError::into_inner).get(target).cloned()
    }

    /// Add the endpoint serving `target`'s host-mediated exports, refusing an
    /// occupied slot so a registration can never clobber an existing guest's
    /// endpoint.
    ///
    /// The caller must hold the lifecycle write guard (this method takes only
    /// the inner map lock, so taking the gate here would deadlock).
    pub(crate) fn insert(&self, target: &GuestId, server: Arc<InProcServer>) -> Result<()> {
        let inserted = {
            let mut servers = self.servers.write().unwrap_or_else(PoisonError::into_inner);
            match servers.entry(target.clone()) {
                Entry::Occupied(_) => false,
                Entry::Vacant(slot) => {
                    slot.insert(server);
                    true
                }
            }
        };
        ensure!(inserted, "guest `{target}` already has an in-process endpoint");
        Ok(())
    }

    /// Remove `target`'s endpoint; in-flight invocations hold their own
    /// server [`Arc`] and complete.
    ///
    /// The caller must hold the lifecycle write guard.
    pub(crate) fn remove(&self, target: &GuestId) {
        self.servers.write().unwrap_or_else(PoisonError::into_inner).remove(target);
    }
}

/// Per-store wRPC view state.
///
/// `wrpc-wasmtime` requires each guest store to expose a [`WrpcCtx`] (a client
/// handle plus a shared-resource table). Omnia's host-mediated dispatch reaches
/// targets through the bound transport carrier, *not* through this client, so
/// the client here is an inert single-use handle that is never invoked — it
/// exists only to satisfy the trait bound and carry the shared-resource table.
pub struct WrpcState {
    client: InProcClient,
    shared: SharedResourceTable,
}

impl WrpcState {
    /// Create fresh per-store wRPC view state.
    #[must_use]
    pub fn new() -> Self {
        // A dummy pipe whose server half is dropped immediately: this client is
        // never invoked (dispatch uses the carrier), so it never reads or writes.
        let (client, _server) = Oneshot::duplex(1);
        Self {
            client,
            shared: SharedResourceTable::default(),
        }
    }

    /// Borrow this state as a [`WrpcCtxView`] paired with the store's resource
    /// table — the shape `wrpc-wasmtime`'s [`wrpc_wasmtime::WrpcView`] returns.
    pub fn view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WrpcCtxView<'a, InProcClient> {
        WrpcCtxView { ctx: self, table }
    }
}

impl Default for WrpcState {
    fn default() -> Self {
        Self::new()
    }
}

impl WrpcCtx<InProcClient> for WrpcState {
    fn context(&self) {}

    fn client(&self) -> &InProcClient {
        &self.client
    }

    fn shared_resources(&mut self) -> &mut SharedResourceTable {
        &mut self.shared
    }
}

impl LinkTransport for InProcess {
    type Client = InProcClient;

    fn connect(&self, target: &GuestId) -> Result<Self::Client> {
        let server = self.server(target).with_context(|| {
            format!(
                "no in-process endpoint serves guest `{target}` (is it registered and does it \
                     export the linked interface?)"
            )
        })?;

        // A fresh pipe per call: the client half drives this invocation; the
        // server half is accepted onto the target's wRPC server, which
        // instantiates the guest fresh (instance-per-call).
        let (client, server_stream) = Oneshot::duplex(DUPLEX_BUF);
        let (server_rx, server_tx) = split(server_stream);
        tokio::spawn(async move {
            if let Err(error) = server.accept((), server_tx, server_rx).await {
                tracing::error!(%error, "in-process link accept failed");
            }
        });

        Ok(client)
    }
}
