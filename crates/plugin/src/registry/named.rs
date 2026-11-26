//! The fetch of a release from the registry a load names, held to the
//! loading guest's grants at every hop.
//!
//! The fetch makes each request itself, over a client built for it alone.
//! Its requests are these:
//!
//! - the registry's metadata;
//! - the manifest and layer, from the host the metadata names;
//! - a token, from the realm a `401` names;
//! - every redirect any of them answers with.
//!
//! The host of each is held to the grants before anything connects to it. The
//! deployment's `[registry."<name>".oci]` settings are the only credentials a
//! request carries.

use std::error::Error;
use std::fmt::{self, Display};
use std::iter;

use http_auth::ChallengeParser;
use omnia_core::{AcquireError, Digest, Grants};
use reqwest::header::{ACCEPT, WWW_AUTHENTICATE};
use reqwest::{RequestBuilder, Response, StatusCode, Url, redirect};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use wasm_pkg_client::oci::client::ClientProtocol;
use wasm_pkg_client::oci::{BasicCredentials, OciRegistryConfig};
use wasm_pkg_client::{Config, Registry, RegistryMetadata};

use crate::store::Reference;

const METADATA_PATH: &str = "/.well-known/wasm-pkg/registry.json";
const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const CONFIG_MEDIA_TYPE: &str = "application/vnd.wasm.config.v0+json";

pub struct Named<'a> {
    registry: &'a Registry,
    grants: &'a Grants,
    client: reqwest::Client,
    protocol: ClientProtocol,
    credentials: Option<BasicCredentials>,
    // the deployment names the `oci` backend, which outranks the metadata's
    // preferred protocol
    oci_configured: bool,
}

impl<'a> Named<'a> {
    pub fn new(
        config: &Config, registry: &'a Registry, grants: &'a Grants,
    ) -> Result<Self, AcquireError> {
        let settings = config.registry_config(registry);
        let backend = settings.and_then(|settings| settings.default_backend());
        if let Some(backend) = backend.filter(|backend| *backend != "oci") {
            return Err(AcquireError::Unavailable(format!(
                "registry `{registry}` is configured with the `{backend}` backend, and a fetch \
                 from a registry a load names speaks `oci` alone"
            )));
        }
        let OciRegistryConfig {
            client_config,
            credentials,
        } = settings
            .map(OciRegistryConfig::try_from)
            .transpose()
            .map_err(|error| {
                AcquireError::Unavailable(format!(
                    "reading the `oci` settings of registry `{registry}`: {error}"
                ))
            })?
            .unwrap_or_default();

        let roots = client_config
            .extra_root_certificates
            .iter()
            .map(reqwest::Certificate::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                AcquireError::Unavailable(format!(
                    "reading the root certificates of registry `{registry}`: {error}"
                ))
            })?;
        ensure_crypto_provider();
        let client = reqwest::Client::builder()
            .redirect(held_redirects(grants.clone()))
            .tls_certs_merge(roots)
            .tls_danger_accept_invalid_certs(client_config.accept_invalid_certificates)
            .build()
            .map_err(|error| {
                AcquireError::Unavailable(format!(
                    "building the client for registry `{registry}`: {error}"
                ))
            })?;

        Ok(Self {
            registry,
            grants,
            client,
            protocol: client_config.protocol,
            credentials,
            oci_configured: backend.is_some(),
        })
    }

    // The release's one layer and the digest its manifest names for it.
    pub async fn fetch(&self, reference: &Reference) -> Result<(Vec<u8>, Digest), AcquireError> {
        let (host, prefix) = self.locate().await?;
        let package = reference.package();
        let repository =
            format!("{}{}/{}", prefix.unwrap_or_default(), package.namespace(), package.name());
        let base = format!("{}://{host}/v2/{repository}", self.scheme(&host));

        // resolve the release to its one layer
        let url = parse(&format!("{base}/manifests/{}", reference.version()))?;
        let (response, auth) = self.manifest(&url, &repository).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(AcquireError::NotFound(format!(
                "resolving `{reference}`: `{url}` answered {}",
                response.status()
            )));
        }
        let body = read(response, &url)
            .await
            .map_err(|why| AcquireError::Unavailable(format!("resolving `{reference}`: {why}")))?;
        let layer = serde_json::from_slice::<Manifest>(&body)
            .map_err(|error| error.to_string())
            .and_then(Manifest::layer)
            .map_err(|why| {
                AcquireError::Unavailable(format!(
                    "resolving `{reference}`: `{url}` is not a wasm component's manifest: {why}"
                ))
            })?;
        let expected: Digest = layer.parse().map_err(|error| {
            AcquireError::Unsupported(format!(
                "the registry digest for `{reference}` is unsupported: {error}"
            ))
        })?;

        // then the layer, under the manifest's authorization
        let url = parse(&format!("{base}/blobs/{expected}"))?;
        let response = self.send(auth.apply(self.client.get(url.clone()))).await?;
        let bytes = read(response, &url)
            .await
            .map_err(|why| AcquireError::Unavailable(format!("fetching `{reference}`: {why}")))?;
        Ok((bytes, expected))
    }

    // The OCI host the registry's metadata names, else the registry itself,
    // and the prefix its repositories take. Metadata that is missing or fails
    // to load leaves the defaults, as wasm-pkg-client's does, but a hop the
    // grants refuse fails the fetch.
    async fn locate(&self) -> Result<(String, Option<String>), AcquireError> {
        let scheme = match self.registry.host() {
            "localhost" | "127.0.0.1" => "http",
            _ => "https",
        };
        let url = parse(&format!("{scheme}://{}{METADATA_PATH}", self.registry))?;
        let metadata = match self.send(self.client.get(url)).await {
            Ok(response) if response.status().is_success() => response
                .bytes()
                .await
                .ok()
                .and_then(|body| serde_json::from_slice::<RegistryMetadata>(&body).ok())
                .unwrap_or_default(),
            Ok(_) | Err(AcquireError::Unavailable(_)) => RegistryMetadata::default(),
            Err(error) => return Err(error),
        };

        if !self.oci_configured
            && let Some(protocol) =
                metadata.preferred_protocol().filter(|protocol| *protocol != "oci")
        {
            return Err(AcquireError::Unavailable(format!(
                "registry `{}` prefers the `{protocol}` protocol, and a fetch from a registry a \
                 load names speaks `oci` alone",
                self.registry
            )));
        }
        let oci = metadata
            .protocol_config::<OciMetadata>("oci")
            .map_err(|error| {
                AcquireError::Unavailable(format!(
                    "reading the metadata of registry `{}`: {error}",
                    self.registry
                ))
            })?
            .unwrap_or_default();
        let host = oci.registry.unwrap_or_else(|| self.registry.to_string());

        // HACK: oci-client's `Reference::resolve_registry`, since Docker Hub
        // serves the distribution API from `index.docker.io`
        let host = if host == "docker.io" { "index.docker.io".to_owned() } else { host };
        Ok((host, oci.namespace_prefix))
    }

    // `ClientProtocol::scheme_for`, which oci-client keeps private.
    fn scheme(&self, host: &str) -> &'static str {
        match &self.protocol {
            ClientProtocol::Http => "http",
            ClientProtocol::HttpsExcept(exceptions)
                if exceptions.iter().any(|exception| exception == host) =>
            {
                "http"
            }
            ClientProtocol::Https | ClientProtocol::HttpsExcept(_) => "https",
        }
    }

    // The manifest, asked for anonymously first. A `401` says how the
    // registry authenticates, and one retry authenticates so.
    async fn manifest(
        &self, url: &Url, repository: &str,
    ) -> Result<(Response, Auth<'_>), AcquireError> {
        let request = |auth: &Auth| {
            auth.apply(self.client.get(url.clone()).header(ACCEPT, MANIFEST_MEDIA_TYPE))
        };
        let response = self.send(request(&Auth::Anonymous)).await?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok((response, Auth::Anonymous));
        }
        let auth = self.authenticate(&response, repository).await?;
        let response = self.send(request(&auth)).await?;
        Ok((response, auth))
    }

    // A Bearer challenge's token, which its realm issues to the deployment's
    // credentials when they are set. Under any other challenge, the
    // credentials themselves.
    async fn authenticate(
        &self, unauthorized: &Response, repository: &str,
    ) -> Result<Auth<'_>, AcquireError> {
        let challenge = unauthorized
            .headers()
            .get(WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
            .and_then(bearer);
        let Some((realm, service)) = challenge else {
            return self.credentials.as_ref().map(Auth::Basic).ok_or_else(|| {
                AcquireError::Unavailable(format!(
                    "registry `{}` asks for credentials, and the deployment sets none for it",
                    self.registry
                ))
            });
        };

        let mut url = parse(&realm)?;
        url.query_pairs_mut()
            .append_pair("scope", &format!("repository:{repository}:pull"))
            .extend_pairs(service.iter().map(|service| ("service", service)));
        let basic = self.credentials.as_ref().map_or(Auth::Anonymous, Auth::Basic);
        let response = self.send(basic.apply(self.client.get(url.clone()))).await?;
        let body = read(response, &url).await.map_err(|why| {
            AcquireError::Unavailable(format!(
                "authenticating to registry `{}`: {why}",
                self.registry
            ))
        })?;
        serde_json::from_slice::<Token>(&body)
            .ok()
            .and_then(|token| token.token.or(token.access_token))
            .map(Auth::Bearer)
            .ok_or_else(|| AcquireError::Unavailable(format!("the realm `{url}` issued no token")))
    }

    // Every request the fetch makes passes here, so its host is held to the
    // grants before anything connects, and the client's policy holds each
    // redirect after.
    async fn send(&self, request: RequestBuilder) -> Result<Response, AcquireError> {
        let request = request
            .build()
            .map_err(|error| AcquireError::Unavailable(format!("building a request: {error}")))?;
        let url = request.url().clone();
        let host = url.host_str().unwrap_or_default();
        if !self.grants.allows_host(host) {
            return Err(self.leads_to(host));
        }
        self.client.execute(request).await.map_err(|error| {
            if let Some(host) = redirect_denied(&error) {
                return self.leads_to(host);
            }
            AcquireError::Unavailable(format!(
                "fetching `{url}`: {:#}",
                anyhow::Error::new(error.without_url())
            ))
        })
    }

    fn leads_to(&self, host: &str) -> AcquireError {
        AcquireError::Denied(format!(
            "registry `{}` leads to `{host}`, a host the loading guest's grants do not reach",
            self.registry
        ))
    }
}

// What a request authenticates as: nobody, the deployment's credentials for
// the registry, or the token the registry's realm issued.
enum Auth<'c> {
    Anonymous,
    Basic(&'c BasicCredentials),
    Bearer(String),
}

impl Auth<'_> {
    fn apply(&self, request: RequestBuilder) -> RequestBuilder {
        match self {
            Self::Anonymous => request,
            Self::Basic(credentials) => request
                .basic_auth(&credentials.username, Some(credentials.password.expose_secret())),
            Self::Bearer(token) => request.bearer_auth(token),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OciMetadata {
    registry: Option<String>,
    namespace_prefix: Option<String>,
}

#[derive(Deserialize)]
struct Token {
    token: Option<String>,
    access_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    media_type: Option<String>,
    config: Descriptor,
    layers: Vec<Descriptor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Descriptor {
    media_type: String,
    digest: String,
}

impl Manifest {
    // The digest of a wasm component's one layer, held to the checks
    // oci-wasm makes of a manifest it pulls.
    fn layer(self) -> Result<String, String> {
        if self.media_type.as_deref() != Some(MANIFEST_MEDIA_TYPE) {
            return Err(format!("its media type is not `{MANIFEST_MEDIA_TYPE}`"));
        }
        if self.config.media_type != CONFIG_MEDIA_TYPE {
            return Err(format!("its config's media type is not `{CONFIG_MEDIA_TYPE}`"));
        }
        match <[Descriptor; 1]>::try_from(self.layers) {
            Ok([layer]) => Ok(layer.digest),
            Err(layers) => Err(format!("it has {} layers, not one", layers.len())),
        }
    }
}

// The realm and service of the first Bearer challenge in a
// `WWW-Authenticate` value.
fn bearer(header: &str) -> Option<(String, Option<String>)> {
    let challenge = ChallengeParser::new(header)
        .filter_map(Result::ok)
        .find(|challenge| challenge.scheme.eq_ignore_ascii_case("Bearer"))?;
    let param = |name: &str| {
        challenge
            .params
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.to_unescaped())
    };
    Some((param("realm")?, param("service")))
}

// The body of a `2xx` answer from `url`; any other status fails, naming it.
async fn read(response: Response, url: &Url) -> Result<Vec<u8>, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!("`{url}` answered {status}"));
    }
    let body = response.bytes().await.map_err(|error| {
        format!("reading `{url}`: {:#}", anyhow::Error::new(error.without_url()))
    })?;
    Ok(body.into())
}

fn parse(url: &str) -> Result<Url, AcquireError> {
    Url::parse(url)
        .map_err(|error| AcquireError::Unavailable(format!("`{url}` is not a URL: {error}")))
}

// reqwest is built with `rustls-no-provider`, so a process provider must be
// installed before a client is built. One the embedder installed wins.
fn ensure_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

// reqwest's default policy past the host check, so its ten-hop limit holds
fn held_redirects(grants: Grants) -> redirect::Policy {
    redirect::Policy::custom(move |attempt| {
        let host = attempt.url().host_str().unwrap_or_default().to_owned();
        if grants.allows_host(&host) {
            redirect::Policy::default().redirect(attempt)
        } else {
            attempt.error(RedirectDenied(host))
        }
    })
}

fn redirect_denied(error: &reqwest::Error) -> Option<&str> {
    iter::successors(Some(error as &(dyn Error + 'static)), |&error| error.source())
        .find_map(|error| error.downcast_ref::<RedirectDenied>())
        .map(|denied| denied.0.as_str())
}

#[derive(Debug)]
struct RedirectDenied(String);

impl Display for RedirectDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a redirect to `{}` is a host the loading guest's grants do not reach", self.0)
    }
}

impl Error for RedirectDenied {}
