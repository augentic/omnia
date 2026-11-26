//! The loopback registry the suites fetch a release from, by the name a load
//! gives it.
//!
//! It plays an ordered script of exchanges. Each names the host a request
//! reaches it by, the path, and the `Authorization` the request carries. A
//! request off the script is answered `500` and fails [`Loopback::finish`].

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, HOST, HeaderName};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use omnia::Digest;
use tokio::net::TcpListener;

/// The path a registry serves its wasm-pkg metadata at.
pub const METADATA: &str = "/.well-known/wasm-pkg/registry.json";
/// The registry's host name, which a `hosts` grant can list.
pub const NAME: &str = "localhost";
/// The registry's address, which no `hosts` grant reaches.
pub const ADDRESS: &str = "127.0.0.1";

const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

/// A reply the registry gives.
pub type Reply = Response<Full<Bytes>>;

/// A loopback registry, reached by [`NAME`] or [`ADDRESS`] on one port.
#[derive(Clone)]
pub struct Loopback {
    port: u16,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    script: VecDeque<Exchange>,
    faults: Vec<String>,
}

impl Loopback {
    /// Serve on an ephemeral loopback port for the rest of the test.
    pub async fn serve() -> Self {
        let listener = TcpListener::bind((ADDRESS, 0)).await.expect("binding the registry");
        let port = listener.local_addr().expect("the registry's address").port();
        let registry = Self {
            port,
            state: Arc::default(),
        };

        let serving = registry.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let registry = serving.clone();
                let service = service_fn(move |request| {
                    let reply = registry.answer(&request);
                    async move { Ok::<_, Infallible>(reply) }
                });
                tokio::spawn(http1::Builder::new().serve_connection(TokioIo::new(stream), service));
            }
        });
        registry
    }

    /// The registry reached by `host`, as `<host>:<port>`.
    pub fn authority(&self, host: &str) -> String {
        format!("{host}:{}", self.port)
    }

    /// The name a load gives the registry, `localhost:<port>`.
    pub fn name(&self) -> String {
        self.authority(NAME)
    }

    /// The URL of `path` on the registry, reached by `host`.
    pub fn url(&self, host: &str, path: &str) -> String {
        format!("http://{}{path}", self.authority(host))
    }

    /// The `registries` settings under which a fetch speaks plain HTTP to the registry.
    pub fn toml(&self) -> String {
        format!("[registry.\"{}\".oci]\nprotocol = \"http\"\n", self.name())
    }

    /// Replace the script with `exchanges`, played in order.
    pub fn script(&self, exchanges: impl IntoIterator<Item = Exchange>) {
        let mut state = self.state.lock().expect("the script is whole");
        state.script = exchanges.into_iter().collect();
        state.faults.clear();
    }

    /// Assert that every scripted exchange was made and that nothing else arrived.
    pub fn finish(&self) {
        let mut state = self.state.lock().expect("the script is whole");
        let faults = std::mem::take(&mut state.faults);
        let unmade: Vec<String> = state
            .script
            .drain(..)
            .map(|exchange| format!("{} {}", exchange.host, exchange.path))
            .collect();
        drop(state);
        assert!(faults.is_empty(), "requests off the script: {faults:#?}");
        assert!(unmade.is_empty(), "scripted exchanges never made: {unmade:#?}");
    }

    fn answer(&self, request: &Request<Incoming>) -> Reply {
        let header = |name: HeaderName| request.headers().get(name)?.to_str().ok();
        let host = header(HOST).unwrap_or_default();
        let path = request.uri().path();
        let authorization = header(AUTHORIZATION);

        let mut state = self.state.lock().expect("the script is whole");
        let fault = match state.script.pop_front() {
            Some(exchange)
                if host == self.authority(exchange.host)
                    && path == exchange.path
                    && authorization == exchange.authorization.as_deref() =>
            {
                return exchange.reply;
            }
            Some(exchange) => format!(
                "{host} {path} {authorization:?}, where the script expects {} {} {:?}",
                self.authority(exchange.host),
                exchange.path,
                exchange.authorization
            ),
            None => format!("{host} {path} {authorization:?}, past the script's end"),
        };
        state.faults.push(fault);
        drop(state);
        reply(StatusCode::INTERNAL_SERVER_ERROR, &[], "")
    }
}

/// One request the script expects and the reply it gets.
pub struct Exchange {
    /// The host the request reaches the registry by, [`NAME`] or [`ADDRESS`].
    pub host: &'static str,
    /// The request's path, without its query.
    pub path: String,
    /// The `Authorization` the request carries, if any.
    pub authorization: Option<String>,
    /// The registry's reply.
    pub reply: Reply,
}

impl Exchange {
    /// A request for `path` by `host`, carrying no `Authorization`, answered with `reply`.
    pub fn new(host: &'static str, path: impl Into<String>, reply: Reply) -> Self {
        Self {
            host,
            path: path.into(),
            authorization: None,
            reply,
        }
    }
}

/// A reply with `status`, `headers`, and `body`.
pub fn reply(status: StatusCode, headers: &[(&str, &str)], body: impl Into<Bytes>) -> Reply {
    let mut reply = Response::builder().status(status);
    for (name, value) in headers {
        reply = reply.header(*name, *value);
    }
    reply.body(Full::new(body.into())).expect("a well-formed reply")
}

/// A `404`, as from a registry that serves no wasm-pkg metadata.
pub fn not_found() -> Reply {
    reply(StatusCode::NOT_FOUND, &[], "")
}

/// A `307` to `location`.
pub fn redirect(location: &str) -> Reply {
    reply(StatusCode::TEMPORARY_REDIRECT, &[("location", location)], "")
}

/// The manifest of a wasm component whose one layer is `content`.
pub fn manifest(content: &[u8]) -> Reply {
    let config = b"{}";
    let manifest = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST_MEDIA_TYPE,
        "config": {
            "mediaType": "application/vnd.wasm.config.v0+json",
            "digest": Digest::of(config).to_string(),
            "size": config.len(),
        },
        "layers": [{
            "mediaType": "application/wasm",
            "digest": Digest::of(content).to_string(),
            "size": content.len(),
        }],
    });
    reply(StatusCode::OK, &[("content-type", MANIFEST_MEDIA_TYPE)], manifest.to_string())
}
