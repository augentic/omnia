//! A published `omnia:*` package, and the checks on every `wit` dir.
//!
//! A package is encoded with every `@unstable` feature, since the gates travel
//! with it and a consumer enables its own. Its description is the first
//! paragraph of its package docs, and the rest of its metadata is its crate's.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use wasm_metadata::{
    AddMetadata, AddMetadataField, Authors, Description, Homepage, Licenses, Revision, Source,
    Version,
};
use wit_parser::{PackageId, PackageName, Resolve};

use crate::workspace::{Workspace, wit_files};

pub struct Package {
    pub dir: PathBuf,
    pub name: PackageName,
    resolve: Resolve,
    id: PackageId,
}

impl Package {
    pub fn parse(dir: &Path) -> Result<Self> {
        let (resolve, id) = resolve(dir, true)?;
        let name = resolve.packages[id].name.clone();
        Ok(Self {
            dir: dir.to_owned(),
            name,
            resolve,
            id,
        })
    }

    pub fn has_imports(&self) -> bool {
        self.resolve.packages[self.id].worlds.contains_key("imports")
    }

    pub fn encode(&self, workspace: &Workspace) -> Result<Vec<u8>> {
        let docs = self.resolve.packages[self.id].docs.contents.as_deref().unwrap_or_default();
        let description = summary(docs);
        ensure!(
            !description.is_empty(),
            "{} has no package docs: add a `///` summary above its `package` line",
            self.name
        );
        let version =
            self.name.version.as_ref().with_context(|| format!("{} has no version", self.name))?;
        let krate = workspace.crate_of(&self.dir)?;

        let mut metadata = AddMetadata::default();
        metadata.description = AddMetadataField::Set(Description::new(description));
        metadata.version = AddMetadataField::Set(Version::new(version.to_string()));
        if !krate.authors.is_empty() {
            metadata.authors = AddMetadataField::Set(Authors::new(krate.authors.join(", ")));
        }
        if let Some(license) = &krate.license {
            metadata.licenses = AddMetadataField::Set(Licenses::new(license)?);
        }
        if let Some(repository) = &krate.repository {
            metadata.source = AddMetadataField::Set(Source::new(repository)?);
        }
        if let Some(homepage) = &krate.homepage {
            metadata.homepage = AddMetadataField::Set(Homepage::new(homepage)?);
        }
        if let Some(revision) = &workspace.revision {
            metadata.revision = AddMetadataField::Set(Revision::new(revision.clone()));
        }
        metadata.to_wasm(&wit_component::encode(&self.resolve, self.id, false)?)
    }
}

// the first paragraph of `docs` on one line, its code spans unmarked
fn summary(docs: &str) -> String {
    let paragraph = docs.split("\n\n").next().unwrap_or_default();
    paragraph.split_whitespace().collect::<Vec<_>>().join(" ").replace('`', "")
}

// each line of a published package that a plain `//` comment opens, which
// wit-parser would publish as the next item's docs
pub fn comments(dirs: &[PathBuf]) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    for dir in dirs {
        for file in wit_files(dir)? {
            let text = fs::read_to_string(&file)
                .with_context(|| format!("cannot read {}", file.display()))?;
            for (n, line) in text.lines().enumerate() {
                if line.trim_start().strip_prefix("//").is_some_and(|rest| !rest.starts_with('/')) {
                    lines.push(format!("{}:{}:{line}", file.display(), n + 1));
                }
            }
        }
    }
    Ok(lines)
}

// how `dir` fails to resolve for a consumer that enables no features, and for
// one that enables them all
pub fn unresolved(dir: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    for (all_features, features) in [(false, "no features"), (true, "every feature")] {
        if let Err(err) = resolve(dir, all_features) {
            failures.push(format!("{} does not resolve with {features}: {err:#}", dir.display()));
        }
    }
    failures
}

fn resolve(dir: &Path, all_features: bool) -> Result<(Resolve, PackageId)> {
    let mut resolve = Resolve {
        all_features,
        ..Resolve::default()
    };
    let (id, _) = resolve.push_dir(dir).map_err(|err| anyhow!(resolve.render_error(&err)))?;
    Ok((resolve, id))
}
