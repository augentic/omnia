//! The `omnia:sdk` aggregate that the `omnia-sdk` guest bindings generate from.
//!
//! The aggregate is never published. Its `imports` world includes each
//! published package's `imports` world, beside the WASI packages the locked
//! `wasip3` ships. The bindings follow `wasi-rs`'s `ci/regenerate.sh`: one
//! `imports.rs` for every import, and one file per export world whose import
//! types point back at it. Every `@unstable` feature is enabled, since every
//! host links the gated interfaces.

use std::collections::HashSet;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use wit_bindgen_core::wit_parser::{
    PackageId, PackageName, Resolve, SourceMap, UnresolvedPackageGroup,
};
use wit_bindgen_core::{Files, WorldGenerator};
use wit_bindgen_rust::{Opts, WithOption};

use crate::package::Package;
use crate::workspace::{Workspace, wit_files};

pub struct Sdk {
    resolve: Resolve,
    id: PackageId,
}

impl Sdk {
    pub fn new(workspace: &Workspace, packages: &[Package], all_features: bool) -> Result<Self> {
        // each WASI package, then each published package alone
        let mut deps = Vec::new();
        for file in wit_files(&workspace.wasi)? {
            let mut map = SourceMap::default();
            map.push_file(&file)?;
            deps.push(map.parse().map_err(|(map, err)| anyhow!(err.render(&map)))?);
        }
        for package in packages {
            deps.push(UnresolvedPackageGroup::parse_dir(&package.dir)?);
        }

        // the aggregate over them
        let names: Vec<_> = deps.iter().map(|dep| &dep.main.name).collect();
        let wit = aggregate(&workspace.sdk_version, packages, &names)?;
        let main = UnresolvedPackageGroup::parse("sdk.wit", &wit)
            .map_err(|(map, err)| anyhow!(err.render(&map)))?;
        let mut resolve = Resolve {
            all_features,
            ..Resolve::default()
        };
        let id = resolve
            .push_groups(main, deps)
            .map_err(|err| anyhow!(resolve.render_error(&err.into())))?;
        Ok(Self { resolve, id })
    }

    pub fn bindings(&self) -> Result<Bindings> {
        let names: Vec<_> = self.resolve.package_names.keys().collect();
        let sdk = &self.resolve.packages[self.id].name;
        let version = sdk.version.as_ref().with_context(|| format!("{sdk} has no version"))?;
        let suffix = format!("rust-omnia-sdk-{version}");
        let mut files = Files::default();

        let filesystem = find(&names, "wasi", "filesystem")?;
        let clocks = find(&names, "wasi", "clocks")?;
        self.generate(
            "imports",
            Opts {
                generate_all: true,
                with: vec![
                    remap(filesystem, "types", "wasip3::filesystem::types"),
                    remap(clocks, "system-clock", "wasip3::clocks::system_clock"),
                    remap(clocks, "types", "wasip3::clocks::types"),
                ],
                type_section_suffix: Some(suffix.clone()),
                ..Opts::default()
            },
            &mut files,
        )?;

        let types = find(&names, "omnia", "types")?;
        for world in ["messaging", "websocket"] {
            self.generate(
                world,
                Opts {
                    with: vec![
                        remap(types, "types", "crate::bindings::imports::omnia::types::types"),
                        remap(
                            find(&names, "omnia", world)?,
                            "types",
                            &format!("crate::bindings::imports::omnia::{world}::types"),
                        ),
                    ],
                    type_section_suffix: Some(format!("{suffix}-{world}-world")),
                    default_bindings_module: Some("$crate".to_owned()),
                    pub_export_macro: true,
                    export_macro_name: Some(format!("_export_{world}")),
                    ..Opts::default()
                },
                &mut files,
            )?;
        }
        Ok(Bindings(files))
    }

    pub fn targets(&self, guest: &[u8]) -> Result<()> {
        let world = self.resolve.select_world(&[self.id], Some("command"))?;
        wit_component::targets(&self.resolve, world, guest, false)
            .context("the guest does not target `omnia:sdk/command`")
    }

    // `world` as `wit-bindgen rust --format --runtime-path wasip3::wit_bindgen::rt` generates it
    fn generate(&self, world: &str, opts: Opts, files: &mut Files) -> Result<()> {
        let world = self.resolve.select_world(&[self.id], Some(world))?;
        let opts = Opts {
            format: true,
            runtime_path: Some("wasip3::wit_bindgen::rt".to_owned()),
            ..opts
        };
        opts.build().generate(&mut self.resolve.clone(), world, files)
    }
}

// the WIT of `omnia:sdk`, versioned as the `omnia-sdk` crate is
fn aggregate(version: &str, packages: &[Package], names: &[&PackageName]) -> Result<String> {
    let mut includes = String::new();
    for package in packages.iter().filter(|package| package.has_imports()) {
        writeln!(includes, "  include {};", package.name.interface_id("imports"))?;
    }
    let messaging = find(names, "omnia", "messaging")?.interface_id("incoming-handler");
    let websocket = find(names, "omnia", "websocket")?.interface_id("incoming-handler");
    let cli = find(names, "wasi", "cli")?.interface_id("command");
    Ok(format!(
        "package omnia:sdk@{version};

world imports {{
{includes}}}

world messaging {{
  export {messaging};
}}

world websocket {{
  export {websocket};
}}

world command {{
  include imports;
  include {cli};
}}
"
    ))
}

fn find<'a>(names: &[&'a PackageName], namespace: &str, name: &str) -> Result<&'a PackageName> {
    names
        .iter()
        .copied()
        .find(|package| package.namespace == namespace && package.name == name)
        .with_context(|| format!("no `{namespace}:{name}` package"))
}

fn remap(package: &PackageName, interface: &str, path: &str) -> (String, WithOption) {
    (package.interface_id(interface), WithOption::Path(path.to_owned()))
}

pub struct Bindings(Files);

const BINDINGS: &str = "crates/sdk/src/bindings";

impl Bindings {
    pub fn write(&self) -> Result<()> {
        let root = Path::new(BINDINGS);
        for (name, contents) in self.0.iter() {
            let path = root.join(name);
            fs::write(&path, contents)
                .with_context(|| format!("cannot write {}", path.display()))?;
        }
        for extra in extras(root, self.0.iter().map(|(name, _)| name))? {
            fs::remove_file(&extra)
                .with_context(|| format!("cannot remove {}", extra.display()))?;
        }
        Ok(())
    }

    // each committed file a regeneration would change, and any it would drop
    pub fn stale(&self) -> Vec<String> {
        let root = Path::new(BINDINGS);
        let mut stale = Vec::new();
        for (name, contents) in self.0.iter() {
            let path = root.join(name);
            let committed = fs::read(&path).unwrap_or_default();
            if committed != contents {
                let line = crate::parting_line(
                    &String::from_utf8_lossy(&committed),
                    &String::from_utf8_lossy(contents),
                );
                stale.push(format!(
                    "{} is stale from line {line}: run `mise run bindgen`",
                    path.display()
                ));
            }
        }
        if let Ok(extras) = extras(root, self.0.iter().map(|(name, _)| name)) {
            for path in extras {
                stale.push(format!("{} is not generated: run `mise run bindgen`", path.display()));
            }
        }
        stale
    }
}

fn extras<'a>(root: &Path, generated: impl Iterator<Item = &'a str>) -> Result<Vec<PathBuf>> {
    let generated: HashSet<_> = generated.collect();
    let mut extras = Vec::new();
    scan_extras(root, root, &generated, &mut extras)?;
    Ok(extras)
}

fn scan_extras(
    root: &Path, dir: &Path, generated: &HashSet<&str>, extras: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            scan_extras(root, &path, generated, extras)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("{} is not under {}", path.display(), root.display()))?;
        let Some(name) = relative.to_str() else {
            continue;
        };
        if !generated.contains(name) {
            extras.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::extras;

    // the committed file `wit.sh` caught with `diff -ru` but a name-only stale loop missed
    #[test]
    fn extras_lists_files_bindgen_would_drop() {
        let root = std::env::temp_dir().join(format!("omnia-xtask-extras-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("imports.rs"), b"a").unwrap();
        fs::write(root.join("leftover.rs"), b"b").unwrap();

        let found = extras(&root, ["imports.rs", "messaging.rs"].into_iter()).unwrap();
        assert_eq!(found, vec![root.join("leftover.rs")]);

        let _ = fs::remove_dir_all(&root);
    }
}
