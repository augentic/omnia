//! Descriptor-identity resolution of a WIT `Location` into a [`Place`].

use anyhow::{Context as _, anyhow};
use cap_std::fs::{Metadata, MetadataExt as _};
use omnia_backend::vcs::{Intent, Place};
use omnia_core::MountRegistry;
use wasmtime_wasi::filesystem::Descriptor;

use super::generated::omnia::vcs::types::Location;

pub fn resolve(
    descriptor: &Descriptor, registry: &MountRegistry, location: &Location, intent: Intent,
) -> anyhow::Result<Place> {
    let Descriptor::Dir(dir) = descriptor else {
        return Err(anyhow!("location root must be a directory"));
    };

    let meta = Metadata::from_file(&dir.dir).context("reading lent location root metadata")?;
    let entry = registry
        .match_identity(meta.dev(), meta.ino())
        .context("lent location root is not an authorized mount")?;
    Place::from_mount(entry, &location.subpath, intent)
}
