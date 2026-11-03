//! In-process bridge from `omnia-sdk` capability traits to `omnia-backend`
//! service contracts.

use std::any::Any;
use std::error::Error as StdError;
use std::future::{Future, poll_fn};
use std::sync::Arc;
use std::task::{Poll, ready};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use bytes::Bytes;
use http::{Request, Response};
use http_body::{Body, Frame};
use http_body_util::{BodyExt as _, Full};
use omnia_backend::MountRegistry;
use omnia_backend::blobstore::{BlobStoreClient, ContainerMetadata, ObjectMetadata};
use omnia_backend::config::ConfigClient;
use omnia_backend::docstore::{DocStoreClient, Document, QueryOptions, QueryResult};
use omnia_backend::http::HttpClient;
use omnia_backend::identity::{AccessToken, IdentityClient};
use omnia_backend::keyvalue::{self, KeyValueClient};
use omnia_backend::messaging::{self, MessagingClient};
use omnia_backend::model::{
    self, BoundToolHost, ModelClient, Reply, Request as ModelRequest, SessionClose, ToolCall,
    ToolHost, ToolSession, Workspace,
};
use omnia_backend::sql::{DataType, Row, SqlClient};
use omnia_backend::vault::{self, VaultClient};
use omnia_backend::vcs::{
    self, Change, CloneOptions, Entry, Intent, Merged, Place, Rule, VcsClient,
};
use omnia_backend::websocket::{Event, WebSocketClient};
use omnia_sdk::{
    BlobStore, Broadcast, CasError, Config, DocumentStore, HttpRequest, Identity, Message, Model,
    Publish, StateStore, TableStore, Vault, Vcs,
};

/// An SDK capability served by a client in-process.
///
/// Matches the semantics the `wasm32` default bodies have through the Omnia host.
#[derive(Clone, Debug, Default)]
pub struct Direct<T> {
    /// The underlying service client.
    pub client: T,
    /// The authorized filesystem mounts for lending capabilities (`Model`, `Vcs`).
    pub mounts: Arc<MountRegistry>,
}

impl<T> Direct<T> {
    /// Wrap `client` with an empty [`MountRegistry`].
    #[must_use]
    pub fn new(client: T) -> Self {
        Self {
            client,
            mounts: Arc::default(),
        }
    }

    /// Wrap `client` with a pre-opened [`MountRegistry`].
    #[must_use]
    pub const fn with_mounts(client: T, mounts: Arc<MountRegistry>) -> Self {
        Self { client, mounts }
    }
}

impl<K: KeyValueClient> StateStore for Direct<K> {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let bucket =
            self.client.open_bucket(keyvalue::BUCKET.to_owned()).await.context("opening cache")?;
        bucket.get(key.to_owned()).await.context("reading state from cache")
    }

    async fn set(&self, key: &str, value: &[u8], ttl_secs: Option<u64>) -> Result<()> {
        let bucket =
            self.client.open_bucket(keyvalue::BUCKET.to_owned()).await.context("opening cache")?;
        bucket
            .set(key.to_owned(), value.to_vec(), ttl_secs.map(Duration::from_secs))
            .await
            .context("writing state to cache")
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let bucket =
            self.client.open_bucket(keyvalue::BUCKET.to_owned()).await.context("opening cache")?;
        bucket.delete(key.to_owned()).await.context("deleting entry from cache")
    }

    async fn cas(&self, key: &str, expected: Option<&[u8]>, value: &[u8]) -> Result<(), CasError> {
        let bucket = self
            .client
            .open_bucket(keyvalue::BUCKET.to_owned())
            .await
            .map_err(|e| CasError::Store(format!("{e:#}")))?;
        bucket
            .cas(key.to_owned(), expected.map(<[u8]>::to_vec), value.to_vec())
            .await
            .map_err(|e| CasError::Store(format!("{e:#}")))?
            .map_err(CasError::Conflict)
    }

    async fn increment(&self, key: &str, delta: i64) -> Result<i64> {
        let bucket =
            self.client.open_bucket(keyvalue::BUCKET.to_owned()).await.context("opening cache")?;
        bucket
            .increment(key.to_owned(), delta)
            .await
            .with_context(|| format!("incrementing `{key}`"))
    }
}

impl<S: SqlClient> TableStore for Direct<S> {
    async fn query(
        &self, conn_name: String, query: String, params: Vec<DataType>,
    ) -> Result<Vec<Row>> {
        let conn = self.client.open(conn_name).await.context("failed to open connection")?;
        conn.query(query, params).await.context("query failed")
    }

    async fn exec(&self, conn_name: String, query: String, params: Vec<DataType>) -> Result<u32> {
        let conn = self.client.open(conn_name).await.context("failed to open connection")?;
        conn.exec(query, params).await.context("exec failed")
    }
}

impl<D: DocStoreClient> DocumentStore for Direct<D> {
    async fn get(&self, store: &str, id: &str) -> Result<Option<Document>> {
        self.client.get(store.to_owned(), id.to_owned()).await
    }

    async fn insert(&self, store: &str, doc: &Document) -> Result<()> {
        self.client.insert(store.to_owned(), doc.clone()).await
    }

    async fn put(&self, store: &str, doc: &Document) -> Result<()> {
        self.client.put(store.to_owned(), doc.clone()).await
    }

    async fn delete(&self, store: &str, id: &str) -> Result<bool> {
        self.client.delete(store.to_owned(), id.to_owned()).await
    }

    async fn query(&self, store: &str, options: QueryOptions) -> Result<QueryResult> {
        self.client.query(store.to_owned(), options).await
    }
}

impl<B: BlobStoreClient> BlobStore for Direct<B> {
    async fn get(&self, container: &str, name: &str) -> Result<Option<Vec<u8>>> {
        let ctr =
            self.client.get_container(container.to_owned()).await.context("opening container")?;
        let data = ctr
            .get_data(name.to_owned(), 0, u64::MAX)
            .await
            .context("reading object")?
            .map(Vec::from);
        Ok(data)
    }

    async fn put(&self, container: &str, name: &str, data: &[u8]) -> Result<()> {
        let ctr =
            self.client.get_container(container.to_owned()).await.context("opening container")?;
        ctr.write_data(name.to_owned(), Bytes::copy_from_slice(data))
            .await
            .context("writing object")
    }

    async fn delete(&self, container: &str, name: &str) -> Result<()> {
        let ctr =
            self.client.get_container(container.to_owned()).await.context("opening container")?;
        ctr.delete_object(name.to_owned()).await.context("deleting object")
    }

    async fn list(&self, container: &str) -> Result<Vec<String>> {
        let ctr =
            self.client.get_container(container.to_owned()).await.context("opening container")?;
        ctr.list_objects().await.context("listing objects")
    }

    async fn get_range(
        &self, container: &str, name: &str, start: u64, end: u64,
    ) -> Result<Vec<u8>> {
        let ctr =
            self.client.get_container(container.to_owned()).await.context("opening container")?;
        let bytes = ctr
            .get_data(name.to_owned(), start, end)
            .await
            .context("reading object range")?
            .ok_or_else(|| anyhow!("object not found: {name}"))?;
        Ok(bytes.to_vec())
    }

    async fn object_info(&self, container: &str, name: &str) -> Result<ObjectMetadata> {
        let ctr =
            self.client.get_container(container.to_owned()).await.context("opening container")?;
        ctr.object_info(name.to_owned()).await.context("getting object info")
    }

    async fn create_container(&self, name: &str) -> Result<()> {
        let _ =
            self.client.create_container(name.to_owned()).await.context("creating container")?;
        Ok(())
    }

    async fn delete_container(&self, name: &str) -> Result<()> {
        self.client.delete_container(name.to_owned()).await.context("deleting container")
    }

    async fn container_exists(&self, name: &str) -> Result<bool> {
        self.client.container_exists(name.to_owned()).await.context("checking container existence")
    }

    async fn container_info(&self, container: &str) -> Result<ContainerMetadata> {
        let ctr =
            self.client.get_container(container.to_owned()).await.context("opening container")?;
        ctr.info().context("getting container info")
    }
}

impl<G: MessagingClient> Publish for Direct<G> {
    async fn send(&self, topic: &str, message: &Message) -> Result<()> {
        let endpoint = self.client.endpoint().await.context("connecting to broker")?;
        let mut msg = messaging::Message::new(message.payload.clone());
        topic.clone_into(&mut msg.topic);
        if !message.headers.is_empty() {
            msg.metadata = Some(messaging::Metadata {
                inner: message.headers.clone(),
            });
        }
        endpoint
            .send(topic.to_owned(), msg)
            .await
            .with_context(|| format!("sending message to {topic}"))
    }
}

impl<C: ConfigClient> Config for Direct<C> {
    async fn get(&self, key: &str) -> Result<String> {
        let value = self.client.get(key.to_owned()).await.context("getting configuration")?;
        value.ok_or_else(|| anyhow!("configuration not found"))
    }
}

impl<I: IdentityClient> Identity for Direct<I> {
    async fn token(&self, identity: &str, scopes: &[String]) -> Result<AccessToken> {
        let id = self.client.get_identity(identity.to_owned()).await.context("getting identity")?;
        id.get_token(scopes.to_vec()).await.context("getting access token")
    }
}

impl<V: VaultClient> Vault for Direct<V> {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let locker = self
            .client
            .open_locker(vault::LOCKER.to_owned())
            .await
            .context("opening vault locker")?;
        locker.get(key.to_owned()).await.context("getting secret from vault")
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        let locker = self
            .client
            .open_locker(vault::LOCKER.to_owned())
            .await
            .context("opening vault locker")?;
        locker.set(key.to_owned(), value.to_vec()).await.context("setting secret in vault")
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let locker = self
            .client
            .open_locker(vault::LOCKER.to_owned())
            .await
            .context("opening vault locker")?;
        locker.delete(key.to_owned()).await.context("deleting secret from vault")
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let locker = self
            .client
            .open_locker(vault::LOCKER.to_owned())
            .await
            .context("opening vault locker")?;
        locker.exists(key.to_owned()).await.context("checking secret in vault")
    }

    async fn list_ids(&self) -> Result<Vec<String>> {
        let locker = self
            .client
            .open_locker(vault::LOCKER.to_owned())
            .await
            .context("opening vault locker")?;
        locker.list_ids().await.context("listing secrets in vault")
    }
}

impl<H: HttpClient> HttpRequest for Direct<H> {
    async fn fetch<T>(&self, request: Request<T>) -> Result<Response<Bytes>>
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn StdError + Send + Sync + 'static>>,
    {
        let (parts, body) = request.into_parts();
        let req_bytes = collect_body(body).await.map_err(anyhow::Error::from_boxed)?;
        let req_body = Full::new(Bytes::from(req_bytes))
            .map_err(|infallible| match infallible {})
            .boxed_unsync();
        let outbound = Request::from_parts(parts, req_body);

        let response = self.client.fetch(outbound, None).await.context("outbound http fetch")?;
        let (resp_parts, resp_body) = response.into_parts();
        let resp_bytes = resp_body.collect().await.context("reading response body")?.to_bytes();
        Ok(Response::from_parts(resp_parts, resp_bytes))
    }
}

// Frame by frame: `BodyExt::collect` would hold `T::Data`, which the trait
// does not promise is `Send`, across an await.
fn collect_body<T>(
    body: T,
) -> impl Future<Output = Result<Vec<u8>, Box<dyn StdError + Send + Sync>>> + Send
where
    T: Body + Send,
    T::Data: Into<Vec<u8>>,
    T::Error: Into<Box<dyn StdError + Send + Sync + 'static>>,
{
    let mut body = Box::pin(body);
    let mut bytes = Vec::new();
    poll_fn(move |cx| {
        loop {
            match ready!(body.as_mut().poll_frame(cx)) {
                Some(Ok(frame)) => {
                    if let Ok(data) = Frame::into_data(frame) {
                        bytes.extend(data.into());
                    }
                }
                Some(Err(error)) => return Poll::Ready(Err(error.into())),
                None => return Poll::Ready(Ok(std::mem::take(&mut bytes))),
            }
        }
    })
}

impl<W: WebSocketClient> Broadcast for Direct<W> {
    async fn send(&self, _name: &str, data: &[u8], sockets: Option<Vec<String>>) -> Result<()> {
        let endpoint = self.client.endpoint().await.context("connecting to websocket")?;
        endpoint.send(Event::new(data.to_vec()), sockets).await.context("sending websocket event")
    }
}

impl<M: ModelClient> Model for Direct<M> {
    fn complete(
        &self, request: ModelRequest,
    ) -> impl Future<Output = Result<Reply, model::Error>> + Send {
        self.complete_with(request, |call: ToolCall| async move {
            Err::<String, String>(format!(
                "tool `{}` has no handler: answer tool calls with complete_with",
                call.name
            ))
        })
    }

    async fn complete_with<H, F>(
        &self, mut request: ModelRequest, mut handler: H,
    ) -> Result<Reply, model::Error>
    where
        H: FnMut(ToolCall) -> F + Send,
        F: Future<Output = Result<String, String>> + Send,
    {
        request.validate()?;

        let workspace = match request.workspace.clone() {
            None => None,
            Some(path) => {
                let (mount, subpath) = self.mounts.match_path(&path).ok_or_else(|| {
                    model::Error::InvalidRequest(format!(
                        "workspace lend `{path}` matches no preopen"
                    ))
                })?;
                let ws = Workspace::from_mount(mount, subpath)
                    .map_err(|e| model::Error::Backend(format!("{e:#}")))?;
                request.workspace = Some(subpath.to_owned());
                Some(ws)
            }
        };

        let limits = self.client.limits();
        let allowed = request.tool_names();
        let (session, mut calls_rx) = ToolSession::new(limits, allowed);
        let tool_host: Arc<dyn ToolHost> =
            Arc::new(BoundToolHost::new(Arc::clone(&session), workspace));

        let session_for_calls = Arc::clone(&session);
        let calls_loop = async move {
            while let Some(call) = calls_rx.recv().await {
                let id = call.id.clone();
                let output = handler(call).await;
                session_for_calls.deliver(&id, output);
            }
        };

        let session_for_answer = Arc::clone(&session);
        let client = &self.client;
        let answer_fut = async move {
            let _close = SessionClose::new(Arc::clone(&session_for_answer));
            match client.complete(request, tool_host).await {
                Ok(answer) => session_for_answer.take_error().map_or_else(
                    || {
                        Ok(Reply {
                            answer: answer.answer,
                            usage: answer.usage,
                        })
                    },
                    Err,
                ),
                Err(error) => {
                    if let Some(session_err) = session_for_answer.take_error() {
                        return Err(session_err);
                    }
                    match error.downcast::<model::Error>() {
                        Ok(typed) => Err(typed),
                        Err(other) => Err(model::Error::Backend(format!("{other:#}"))),
                    }
                }
            }
        };

        let ((), outcome) = futures::join!(calls_loop, answer_fut);
        outcome
    }
}

impl<V: VcsClient> Direct<V> {
    fn locate(&self, path: &str, intent: Intent) -> Result<Place, vcs::Error> {
        Place::resolve_path(&self.mounts, path, intent).map_err(lower_vcs_error)
    }
}

fn lower_vcs_error(error: anyhow::Error) -> vcs::Error {
    match error.downcast::<vcs::Error>() {
        Ok(typed) => typed,
        Err(other) => vcs::Error::Other(format!("{other:#}")),
    }
}

impl<V: VcsClient> Vcs for Direct<V> {
    async fn resolve(&self, repo: &str, revision: &str) -> Result<String, vcs::Error> {
        let repo = self.locate(repo, Intent::Read)?;
        self.client.resolve(repo, revision.to_owned()).await.map_err(lower_vcs_error)
    }

    async fn descends(
        &self, repo: &str, ancestor: &str, descendant: &str,
    ) -> Result<bool, vcs::Error> {
        let repo = self.locate(repo, Intent::Read)?;
        self.client
            .descends(repo, ancestor.to_owned(), descendant.to_owned())
            .await
            .map_err(lower_vcs_error)
    }

    async fn head(&self, at: &str) -> Result<String, vcs::Error> {
        let at = self.locate(at, Intent::Read)?;
        self.client.head(at).await.map_err(lower_vcs_error)
    }

    async fn commit(&self, at: &str, message: &str) -> Result<Option<String>, vcs::Error> {
        let at = self.locate(at, Intent::Mutate)?;
        self.client.commit(at, message.to_owned()).await.map_err(lower_vcs_error)
    }

    async fn merge(
        &self, at: &str, revision: &str, message: &str, policy: &[Rule],
    ) -> Result<Merged, vcs::Error> {
        let at = self.locate(at, Intent::Mutate)?;
        self.client
            .merge(at, revision.to_owned(), message.to_owned(), policy.to_vec())
            .await
            .map_err(lower_vcs_error)
    }

    async fn log(&self, repo: &str, revision: &str, base: &str) -> Result<Vec<Entry>, vcs::Error> {
        let repo = self.locate(repo, Intent::Read)?;
        self.client.log(repo, revision.to_owned(), base.to_owned()).await.map_err(lower_vcs_error)
    }

    async fn init(&self, at: &str) -> Result<(), vcs::Error> {
        let at = self.locate(at, Intent::Create)?;
        self.client.init(at).await.map_err(lower_vcs_error)
    }

    async fn add(&self, repo: &str, at: &str, revision: &str) -> Result<(), vcs::Error> {
        let repo = self.locate(repo, Intent::Mutate)?;
        let at = self.locate(at, Intent::Create)?;
        self.client.add(repo, at, revision.to_owned()).await.map_err(lower_vcs_error)
    }

    async fn remove(&self, at: &str) -> Result<(), vcs::Error> {
        let at = self.locate(at, Intent::Mutate)?;
        self.client.remove(at).await.map_err(lower_vcs_error)
    }

    async fn pending(&self, at: &str) -> Result<Vec<Change>, vcs::Error> {
        let at = self.locate(at, Intent::Read)?;
        self.client.pending(at).await.map_err(lower_vcs_error)
    }

    async fn clone_repo(
        &self, url: &str, at: &str, options: CloneOptions,
    ) -> Result<(), vcs::Error> {
        let at = self.locate(at, Intent::Create)?;
        self.client.clone_repo(url.to_owned(), at, options).await.map_err(lower_vcs_error)
    }

    async fn fetch(&self, repo: &str, remote: &str) -> Result<(), vcs::Error> {
        let repo = self.locate(repo, Intent::Mutate)?;
        self.client.fetch(repo, remote.to_owned()).await.map_err(lower_vcs_error)
    }

    async fn label(&self, repo: &str, name: &str, revision: &str) -> Result<(), vcs::Error> {
        let repo = self.locate(repo, Intent::Mutate)?;
        self.client.label(repo, name.to_owned(), revision.to_owned()).await.map_err(lower_vcs_error)
    }

    async fn labelled(&self, repo: &str, name: &str) -> Result<String, vcs::Error> {
        let repo = self.locate(repo, Intent::Read)?;
        self.client.labelled(repo, name.to_owned()).await.map_err(lower_vcs_error)
    }

    async fn fetched(&self, repo: &str, remote: &str, name: &str) -> Result<String, vcs::Error> {
        let repo = self.locate(repo, Intent::Read)?;
        self.client.fetched(repo, remote.to_owned(), name.to_owned()).await.map_err(lower_vcs_error)
    }

    async fn push(&self, repo: &str, remote: &str, label: &str) -> Result<(), vcs::Error> {
        let repo = self.locate(repo, Intent::Mutate)?;
        self.client.push(repo, remote.to_owned(), label.to_owned()).await.map_err(lower_vcs_error)
    }
}
