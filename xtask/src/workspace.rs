//! The workspace's WIT: the packages it publishes and the copies it keeps.
//!
//! - A published package is a `crates/wasi-*/wit` package, or `crates/plugin/wit`.
//! - A copy is a file under a crate's or an example's `wit/deps`.
//! - A copy's owner is the published package of its name, or the WASI package
//!   the locked `wasip3` ships.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use wit_parser::{PackageName, UnresolvedPackageGroup};

use crate::package::Package;

pub struct Workspace {
    pub sdk_version: String,
    pub wasi: PathBuf,
    pub revision: Option<String>,
    pub published: Vec<PathBuf>,
    pub wit_dirs: Vec<PathBuf>,
    crates: Vec<Crate>,
}

impl Workspace {
    pub fn load() -> Result<Self> {
        // the crates the lock holds
        let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let output = Command::new(cargo)
            .args(["metadata", "--format-version", "1", "--locked"])
            .output()
            .context("cannot run `cargo metadata`")?;
        if !output.status.success() {
            bail!("`cargo metadata` failed: {}", String::from_utf8_lossy(&output.stderr));
        }
        let Metadata { packages: crates } = serde_json::from_slice(&output.stdout)?;
        let sdk =
            crates.iter().find(|krate| krate.name == "omnia-sdk").context("no `omnia-sdk`")?;
        let [wasip3] = crates.iter().filter(|krate| krate.name == "wasip3").collect::<Vec<_>>()[..]
        else {
            bail!("the lock does not hold exactly one `wasip3`");
        };

        // the published dirs, in the order `omnia:sdk` includes them
        let mut published: Vec<_> = entries(Path::new("crates"))?
            .into_iter()
            .filter(|dir| {
                dir.file_name().is_some_and(|name| name.to_string_lossy().starts_with("wasi-"))
            })
            .map(|dir| dir.join("wit"))
            .filter(|dir| dir.is_dir())
            .collect();
        published.push(PathBuf::from("crates/plugin/wit"));

        // every `wit` dir a crate or an example keeps
        let mut wit_dirs = Vec::new();
        for parent in ["crates", "examples"] {
            wit_dirs.extend(
                entries(Path::new(parent))?
                    .into_iter()
                    .map(|dir| dir.join("wit"))
                    .filter(|dir| dir.is_dir()),
            );
        }

        Ok(Self {
            sdk_version: sdk.version.clone(),
            wasi: wasip3.manifest_path.with_file_name("wit").join("deps"),
            revision: revision(),
            published,
            wit_dirs,
            crates,
        })
    }

    pub fn packages(&self) -> Result<Vec<Package>> {
        self.published.iter().map(|dir| Package::parse(dir)).collect()
    }

    pub fn crate_of(&self, dir: &Path) -> Result<&Crate> {
        let manifest = dir.with_file_name("Cargo.toml");
        self.crates
            .iter()
            .find(|krate| krate.manifest_path.ends_with(&manifest))
            .with_context(|| format!("no crate holds {}", dir.display()))
    }

    // each copy that differs from its owner, beside the owner
    pub fn stale_copies(&self) -> Result<Vec<(PathBuf, PathBuf)>> {
        let mut stale = Vec::new();
        for (copy, owner) in self.copies()? {
            if read(&copy)? != read(&owner)? {
                stale.push((copy, owner));
            }
        }
        Ok(stale)
    }

    // the copies it rewrote, beside their owners
    pub fn sync(&self) -> Result<Vec<(PathBuf, PathBuf)>> {
        let stale = self.stale_copies()?;
        for (copy, owner) in &stale {
            fs::write(copy, read(owner)?)
                .with_context(|| format!("cannot write {}", copy.display()))?;
        }
        Ok(stale)
    }

    fn copies(&self) -> Result<Vec<(PathBuf, PathBuf)>> {
        // every published and WASI file, by the package it declares
        let mut owners = Vec::new();
        for dir in self.published.iter().chain([&self.wasi]) {
            for file in wit_files(dir)? {
                owners.push((package_name(&file)?, file));
            }
        }

        // every copy, as wit-parser reads `deps`: `<pkg>.wit` or `<pkg>/*.wit`
        let mut copies = Vec::new();
        for deps in self.wit_dirs.iter().map(|dir| dir.join("deps")).filter(|deps| deps.is_dir()) {
            let mut files = wit_files(&deps)?;
            for dir in entries(&deps)?.into_iter().filter(|dir| dir.is_dir()) {
                files.extend(wit_files(&dir)?);
            }
            for copy in files {
                let name = package_name(&copy)?;
                let (_, owner) = owners
                    .iter()
                    .find(|(owner, _)| owner.namespace == name.namespace && owner.name == name.name)
                    .with_context(|| {
                        format!("no published or WASI package owns {}", copy.display())
                    })?;
                copies.push((copy, owner.clone()));
            }
        }
        copies.sort();
        Ok(copies)
    }
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Crate>,
}

#[derive(Deserialize)]
pub struct Crate {
    name: String,
    version: String,
    manifest_path: PathBuf,
    pub authors: Vec<String>,
    pub license: Option<String>,
    pub repository: Option<String>,
    pub homepage: Option<String>,
}

// the commit the tree is at, when git can say
fn revision() -> Option<String> {
    let output = Command::new("git").args(["rev-parse", "HEAD"]).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn wit_files(dir: &Path) -> Result<Vec<PathBuf>> {
    Ok(entries(dir)?
        .into_iter()
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "wit"))
        .collect())
}

fn entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(dir)
        .and_then(|entries| {
            entries.map(|entry| entry.map(|entry| entry.path())).collect::<Result<Vec<_>, _>>()
        })
        .with_context(|| format!("cannot read {}", dir.display()))?;
    entries.sort();
    Ok(entries)
}

fn package_name(file: &Path) -> Result<PackageName> {
    let text =
        fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let group = UnresolvedPackageGroup::parse(file, &text)
        .map_err(|(map, err)| anyhow!(err.render(&map)))?;
    Ok(group.main.name)
}

fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("cannot read {}", path.display()))
}
