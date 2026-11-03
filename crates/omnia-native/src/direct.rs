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
use omnia_backend::blobstore::{BlobStoreClient, Container, ContainerMetadata, ObjectMetadata};
use omnia_backend::config::ConfigClient;
use omnia_backend::docstore::{DocStoreClient, Document, QueryOptions, QueryResult};
use omnia_backend::http::HttpClient;
use omnia_backend::identity::{AccessToken, IdentityClient};
use omnia_backend::keyvalue::{self, Bucket, KeyValueClient};
use omnia_backend::messaging::{self, MessagingClient};
use omnia_backend::model::{
    self, ModelClient, Reply, Request as ModelRequest, ToolCall, ToolSession, Workspace,
};
use omnia_backend::sql::{DataType, Row, SqlClient};
use omnia_backend::vault::{self, Locker, VaultClient};
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
/// Matches the semantics [`omnia_sdk::WasiProvider`] has through the Omnia host.
#[derive(Clone, Debug, Default)]
pub struct Direct<T> {
    /// The underlying service client.
    pub client: T,
    /// The authorized filesystem mounts for lending capabilities (`Model`, `Vcs`).
    pub mounts: MountRegistry,
}

impl<T> Direct<T> {
    /// Wrap `client` with an empty [`MountRegistry`].
    #[must_use]
    pub const fn new(client: T) -> Self {
        Self {
            client,
            mounts: MountRegistry::new(),
        }
    }

    /// Wrap `client` with a pre-opened [`MountRegistry`].
    #[must_use]
    pub const fn with_mounts(client: T, mounts: MountRegistry) -> Self {
        Self { client, mounts }
    }
}

impl<K: KeyValueClient> Direct<K> {
    async fn bucket(&self) -> Result<Arc<dyn Bucket>> {
        self.client.open_bucket(keyvalue::BUCKET.to_owned()).await.context("opening cache")
    }
}

impl<K: KeyValueClient> StateStore for Direct<K> {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.bucket().await?.get(key.to_owned()).await.context("reading state from cache")
    }

    async fn set(&self, key: &str, value: &[u8], ttl_secs: Option<u64>) -> Result<()> {
        self.bucket()
            .await?
            .set(key.to_owned(), value.to_vec(), ttl_secs.map(Duration::from_secs))
            .await
            .context("writing state to cache")
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.bucket().await?.delete(key.to_owned()).await.context("deleting entry from cache")
    }

    async fn cas(&self, key: &str, expected: Option<&[u8]>, value: &[u8]) -> Result<(), CasError> {
        let bucket = self.bucket().await.map_err(|e| CasError::Store(format!("{e:#}")))?;
        bucket
            .cas(key.to_owned(), expected.map(<[u8]>::to_vec), value.to_vec())
            .await
            .map_err(|e| CasError::Store(format!("{e:#}")))?
            .map_err(CasError::Conflict)
    }

    async fn increment(&self, key: &str, delta: i64) -> Result<i64> {
        self.bucket()
            .await?
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

impl<B: BlobStoreClient> Direct<B> {
    async fn container(&self, name: &str) -> Result<Arc<dyn Container>> {
        self.client.get_container(name.to_owned()).await.context("opening container")
    }
}

impl<B: BlobStoreClient> BlobStore for Direct<B> {
    async fn get(&self, container: &str, name: &str) -> Result<Option<Vec<u8>>> {
        let data = self
            .container(container)
            .await?
            .get_data(name.to_owned(), 0, u64::MAX)
            .await
            .context("reading object")?
            .map(Vec::from);
        Ok(data)
    }

    async fn put(&self, container: &str, name: &str, data: &[u8]) -> Result<()> {
        self.container(container)
            .await?
            .write_data(name.to_owned(), Bytes::copy_from_slice(data))
            .await
            .context("writing object")
    }

    async fn delete(&self, container: &str, name: &str) -> Result<()> {
        self.container(container)
            .await?
            .delete_object(name.to_owned())
            .await
            .context("deleting object")
    }

    async fn list(&self, container: &str) -> Result<Vec<String>> {
        self.container(container).await?.list_objects().await.context("listing objects")
    }

    async fn get_range(
        &self, container: &str, name: &str, start: u64, end: u64,
    ) -> Result<Vec<u8>> {
        let bytes = self
            .container(container)
            .await?
            .get_data(name.to_owned(), start, end)
            .await
            .context("reading object range")?
            .ok_or_else(|| anyhow!("object not found: {name}"))?;
        Ok(bytes.to_vec())
    }

    async fn object_info(&self, container: &str, name: &str) -> Result<ObjectMetadata> {
        self.container(container)
            .await?
            .object_info(name.to_owned())
            .await
            .context("getting object info")
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
        self.container(container).await?.info().context("getting container info")
    }

    async fn has(&self, container: &str, name: &str) -> Result<bool> {
        self.container(container)
            .await?
            .has_object(name.to_owned())
            .await
            .context("checking object existence")
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

impl<V: VaultClient> Direct<V> {
    async fn locker(&self) -> Result<Arc<dyn Locker>> {
        self.client.open_locker(vault::LOCKER.to_owned()).await.context("opening vault locker")
    }
}

impl<V: VaultClient> Vault for Direct<V> {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.locker().await?.get(key.to_owned()).await.context("getting secret from vault")
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        self.locker()
            .await?
            .set(key.to_owned(), value.to_vec())
            .await
            .context("setting secret in vault")
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.locker().await?.delete(key.to_owned()).await.context("deleting secret from vault")
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        self.locker().await?.exists(key.to_owned()).await.context("checking secret in vault")
    }

    async fn list_ids(&self) -> Result<Vec<String>> {
        self.locker().await?.list_ids().await.context("listing secrets in vault")
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
                let (ws, subpath) = Workspace::resolve_path(&self.mounts, &path)?;
                request.workspace = Some(subpath.to_owned());
                Some(ws)
            }
        };

        let (session, mut calls_rx, tool_host) =
            ToolSession::open(self.client.limits(), request.tool_names(), workspace);

        let session_for_calls = Arc::clone(&session);
        let calls_loop = async move {
            while let Some(call) = calls_rx.recv().await {
                let id = call.id.clone();
                let output = handler(call).await;
                session_for_calls.deliver(&id, output);
            }
        };

        let answer_fut = session.finish(self.client.complete(request, tool_host));
        let ((), outcome) = futures::join!(calls_loop, answer_fut);
        outcome
    }
}

impl<V: VcsClient> Direct<V> {
    fn locate(&self, path: &str, intent: Intent) -> Result<Place, vcs::Error> {
        Ok(Place::resolve_path(&self.mounts, path, intent)?)
    }
}

impl<V: VcsClient> Vcs for Direct<V> {
    async fn resolve(&self, repo: &str, revision: &str) -> Result<String, vcs::Error> {
        let repo = self.locate(repo, Intent::Read)?;
        Ok(self.client.resolve(repo, revision.to_owned()).await?)
    }

    async fn descends(
        &self, repo: &str, ancestor: &str, descendant: &str,
    ) -> Result<bool, vcs::Error> {
        let repo = self.locate(repo, Intent::Read)?;
        Ok(self.client.descends(repo, ancestor.to_owned(), descendant.to_owned()).await?)
    }

    async fn head(&self, at: &str) -> Result<String, vcs::Error> {
        let at = self.locate(at, Intent::Read)?;
        Ok(self.client.head(at).await?)
    }

    async fn commit(&self, at: &str, message: &str) -> Result<Option<String>, vcs::Error> {
        let at = self.locate(at, Intent::Mutate)?;
        Ok(self.client.commit(at, message.to_owned()).await?)
    }

    async fn merge(
        &self, at: &str, revision: &str, message: &str, policy: &[Rule],
    ) -> Result<Merged, vcs::Error> {
        let at = self.locate(at, Intent::Mutate)?;
        Ok(self.client.merge(at, revision.to_owned(), message.to_owned(), policy.to_vec()).await?)
    }

    async fn log(&self, repo: &str, revision: &str, base: &str) -> Result<Vec<Entry>, vcs::Error> {
        let repo = self.locate(repo, Intent::Read)?;
        Ok(self.client.log(repo, revision.to_owned(), base.to_owned()).await?)
    }

    async fn init(&self, at: &str) -> Result<(), vcs::Error> {
        let at = self.locate(at, Intent::Create)?;
        Ok(self.client.init(at).await?)
    }

    async fn add(&self, repo: &str, at: &str, revision: &str) -> Result<(), vcs::Error> {
        let repo = self.locate(repo, Intent::Mutate)?;
        let at = self.locate(at, Intent::Create)?;
        Ok(self.client.add(repo, at, revision.to_owned()).await?)
    }

    async fn remove(&self, at: &str) -> Result<(), vcs::Error> {
        let at = self.locate(at, Intent::Mutate)?;
        Ok(self.client.remove(at).await?)
    }

    async fn pending(&self, at: &str) -> Result<Vec<Change>, vcs::Error> {
        let at = self.locate(at, Intent::Read)?;
        Ok(self.client.pending(at).await?)
    }

    async fn clone_repo(
        &self, url: &str, at: &str, options: CloneOptions,
    ) -> Result<(), vcs::Error> {
        let at = self.locate(at, Intent::Create)?;
        Ok(self.client.clone_repo(url.to_owned(), at, options).await?)
    }

    async fn fetch(&self, repo: &str, remote: &str) -> Result<(), vcs::Error> {
        let repo = self.locate(repo, Intent::Mutate)?;
        Ok(self.client.fetch(repo, remote.to_owned()).await?)
    }

    async fn label(&self, repo: &str, name: &str, revision: &str) -> Result<(), vcs::Error> {
        let repo = self.locate(repo, Intent::Mutate)?;
        Ok(self.client.label(repo, name.to_owned(), revision.to_owned()).await?)
    }

    async fn labelled(&self, repo: &str, name: &str) -> Result<String, vcs::Error> {
        let repo = self.locate(repo, Intent::Read)?;
        Ok(self.client.labelled(repo, name.to_owned()).await?)
    }

    async fn fetched(&self, repo: &str, remote: &str, name: &str) -> Result<String, vcs::Error> {
        let repo = self.locate(repo, Intent::Read)?;
        Ok(self.client.fetched(repo, remote.to_owned(), name.to_owned()).await?)
    }

    async fn push(&self, repo: &str, remote: &str, label: &str) -> Result<(), vcs::Error> {
        let repo = self.locate(repo, Intent::Mutate)?;
        Ok(self.client.push(repo, remote.to_owned(), label.to_owned()).await?)
    }
}
