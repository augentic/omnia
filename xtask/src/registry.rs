//! The wasm-pkg registry that `.wkg-config.toml` routes the `omnia:*` packages to.
//!
//! A release is compared with the tree as WIT text, so metadata alone is never
//! a change. Credentials come from `WKG_REGISTRY_GHCR_IO_AUTH_BEARER` or the
//! Docker credential store (`docker login ghcr.io`), as with `wkg`.

use std::io::Cursor;
use std::{fmt, fs};

use anyhow::{Context, Result, anyhow};
use futures::TryStreamExt;
use futures::future::try_join_all;
use tokio::runtime::Runtime;
use wasm_pkg_client::{Client, Config, Error, PackageRef, PublishOpts};
use wit_component::WitPrinter;
use wit_parser::PackageName;

const CONFIG: &str = ".wkg-config.toml";

// a `wasm_pkg_client::Error` prints its source, so each folds into one message
// rather than an anyhow chain that would print it twice
pub struct Registry {
    client: Client,
    runtime: Runtime,
}

impl Registry {
    pub fn open() -> Result<Self> {
        let config = fs::read_to_string(CONFIG).with_context(|| format!("cannot read {CONFIG}"))?;
        let client = Client::new(Config::from_toml(&config)?);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        Ok(Self { client, runtime })
    }

    // each package beside its release, all fetched at once
    pub fn releases(&self, packages: Vec<(PackageName, Vec<u8>)>) -> Result<Vec<Release>> {
        let releases = packages.into_iter().map(|(package, wasm)| self.release(package, wasm));
        self.runtime.block_on(try_join_all(releases))
    }

    pub fn publish(&self, release: &Release) -> Result<()> {
        let data = Box::pin(Cursor::new(release.wasm.clone()));
        self.runtime
            .block_on(self.client.publish_release_data(data, PublishOpts::default()))
            .map_err(|err| anyhow!("cannot publish {}: {err}", release.package))?;
        Ok(())
    }

    async fn release(&self, package: PackageName, wasm: Vec<u8>) -> Result<Release> {
        let unread = |err: Error| {
            anyhow!(
                "{package}: no release could be read through the registry {CONFIG} routes it to: \
                 {err}"
            )
        };
        let reference: PackageRef = format!("{}:{}", package.namespace, package.name).parse()?;
        let version =
            package.version.clone().with_context(|| format!("{package} has no version"))?;

        // absent when the registry lists no such package, or no such version of it
        let versions = match self.client.list_all_versions(&reference).await {
            Ok(versions) => versions,
            Err(err) if registry_absent(&err) => return Ok(Release::absent(package, wasm)),
            Err(err) => return Err(unread(err)),
        };
        if !versions.iter().any(|info| info.version == version) {
            return Ok(Release::absent(package, wasm));
        }

        let release = match self.client.get_release(&reference, &version).await {
            Ok(release) => release,
            Err(err) if registry_absent(&err) => return Ok(Release::absent(package, wasm)),
            Err(err) => return Err(unread(err)),
        };
        let published = self
            .client
            .stream_content(&reference, &release)
            .await
            .map_err(unread)?
            .try_fold(Vec::new(), |mut published, chunk| async move {
                published.extend_from_slice(&chunk);
                Ok(published)
            })
            .await
            .map_err(unread)?;
        let state = State::compare(&wit(&published)?, &wit(&wasm)?);
        Ok(Release { package, wasm, state })
    }
}

// the absent states `wkg get` used to read out of its stderr
fn registry_absent(err: &Error) -> bool {
    match err {
        Error::PackageNotFound | Error::VersionNotFound(_) => true,
        err => {
            let message = format!("{err:#}").to_ascii_lowercase();
            ["manifest unknown", "name unknown", "package not found", "version not found"]
                .iter()
                .any(|needle| message.contains(needle))
        }
    }
}

// the WIT text `wasm` encodes, with the packages it uses nested after it
fn wit(wasm: &[u8]) -> Result<String> {
    let decoded = wit_component::decode(wasm)?;
    let (resolve, main) = (decoded.resolve(), decoded.package());
    let nested: Vec<_> =
        resolve.packages.iter().map(|(id, _)| id).filter(|id| *id != main).collect();
    let mut printer = WitPrinter::default();
    printer.print(resolve, main, &nested)?;
    Ok(printer.output.into())
}

// a package the tree builds, as `wasm`, beside the state of its release
pub struct Release {
    pub package: PackageName,
    pub wasm: Vec<u8>,
    pub state: State,
}

impl Release {
    const fn absent(package: PackageName, wasm: Vec<u8>) -> Self {
        Self {
            package,
            wasm,
            state: State::Absent,
        }
    }
}

impl fmt::Display for Release {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.package, self.state)
    }
}

pub enum State {
    Absent,
    Same,
    Differs { line: usize, published: String, tree: String },
}

impl State {
    fn compare(published: &str, tree: &str) -> Self {
        if published == tree {
            return Self::Same;
        }
        let line = crate::parting_line(published, tree);
        let at = |text: &str| text.lines().nth(line - 1).unwrap_or_default().to_owned();
        Self::Differs {
            line,
            published: at(published),
            tree: at(tree),
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => f.write_str("absent"),
            Self::Same => f.write_str("same"),
            Self::Differs {
                line,
                published,
                tree,
            } => {
                write!(f, "differs at line {line}\n  published: {published}\n  tree:      {tree}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use wasm_pkg_client::{Error, Version};

    use super::registry_absent;

    #[test]
    fn registry_absent_package_and_version_variants() {
        assert!(registry_absent(&Error::PackageNotFound));
        assert!(registry_absent(&Error::VersionNotFound(Version::parse("0.1.0").unwrap())));
    }

    // the needles `scripts/wit.sh` matched in `wkg get` stderr
    #[test]
    fn registry_absent_oci_stderr_needles() {
        for message in
            ["manifest unknown", "NAME UNKNOWN", "Package Not Found", "version not found: 0.1.0"]
        {
            let err = Error::RegistryError(anyhow::anyhow!(message));
            assert!(registry_absent(&err), "{message}");
        }
    }

    #[test]
    fn registry_absent_other_errors() {
        assert!(!registry_absent(&Error::CredentialError(anyhow::anyhow!("denied"))));
        assert!(!registry_absent(&Error::RegistryError(anyhow::anyhow!("connection reset"))));
    }
}
