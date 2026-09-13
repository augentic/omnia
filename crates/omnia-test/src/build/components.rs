//! The `Components` builder: what to compile, how to discover it, and what to
//! generate afterwards.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::env::{self, WASM_TARGET};
use super::render::{Artifact, dep_info_paths, refresh_examples, render_gen};

/// One guest program compiled to a component: an `[[example]]` of one
/// package, or a `cdylib` package of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// The artifact stem, and the `foreach_<group>!` arm's identifier.
    pub name: String,
    /// The generated `pub const` name: the uppercased `name` for an example,
    /// `<GROUP>_<NAME>` for a package.
    pub constant: String,
    /// The `foreach_<group>!` macro this program belongs to.
    pub group: String,
    /// An example's source path relative to the package manifest, or a
    /// package's name as the nested build is told it.
    pub source: String,
    /// Whether the program is an `[[example]]` target — its artifact
    /// uplifted under `debug/examples/` — rather than a `cdylib` package in
    /// the profile directory itself.
    pub example: bool,
}

/// Where programs come from: an explicit example list, a scanned
/// `<group>/<scenario>.rs` tree, or `cdylib` packages — listed, or every
/// crate directory under one parent. A build may draw from several.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Source {
    Examples { names: Vec<String>, group: String },
    Scan(PathBuf),
    Packages { names: Vec<String>, group: String },
    ScanPackages { dir: PathBuf, group: String },
}

/// The group a listed source starts in.
const DEFAULT_GROUP: &str = "examples";

impl Source {
    /// Whether the artifacts are `[[example]]` targets, uplifted under
    /// `debug/examples/` rather than `debug/`.
    const fn is_examples(&self) -> bool {
        matches!(self, Self::Examples { .. } | Self::Scan(_))
    }
}

/// A nested `wasm32-wasip2` build of guest programs, configured from a
/// consumer's `build.rs`.
#[derive(Clone, Debug)]
pub struct Components {
    root: PathBuf,
    package: Option<String>,
    sources: Vec<Source>,
    extras: Vec<String>,
    sync: Option<PathBuf>,
    tracked: Vec<PathBuf>,
}

impl Components {
    /// A build rooted at the workspace `root` relative to the consumer's
    /// manifest directory (typically `"../.."`).
    #[must_use]
    pub fn in_workspace(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            package: None,
            sources: Vec::new(),
            extras: Vec::new(),
            sync: None,
            tracked: Vec::new(),
        }
    }

    /// The workspace package whose examples are built; the root package
    /// otherwise.
    #[must_use]
    pub fn package(mut self, name: impl Into<String>) -> Self {
        self.package = Some(name.into());
        self
    }

    /// An explicit list of `[[example]]` names, one source in the default
    /// group until [`group`](Self::group) renames it.
    #[must_use]
    pub fn examples<S: Into<String>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        self.sources.push(Source::Examples {
            names: names.into_iter().map(Into::into).collect(),
            group: DEFAULT_GROUP.to_owned(),
        });
        self
    }

    /// Discovers programs as `<dir>/<group>/<scenario>.rs` (relative to the
    /// workspace root), named `<group>_<scenario>`.
    #[must_use]
    pub fn scan(mut self, dir: impl AsRef<Path>) -> Self {
        self.sources.push(Source::Scan(dir.as_ref().to_path_buf()));
        self
    }

    /// An explicit list of workspace `cdylib` packages compiled as
    /// components in their own right — the shipped components, not example
    /// stand-ins — one source in the default group until
    /// [`group`](Self::group) renames it. Each is named by its crate name
    /// (`-` becomes `_`) and its constant is `<GROUP>_<NAME>`.
    #[must_use]
    pub fn packages<S: Into<String>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        self.sources.push(Source::Packages {
            names: names.into_iter().map(Into::into).collect(),
            group: DEFAULT_GROUP.to_owned(),
        });
        self
    }

    /// Discovers packages as every `<dir>/<name>/Cargo.toml` (relative to
    /// the workspace root); an entry under `dir` that is not a crate
    /// directory is a layout error. Otherwise as [`packages`](Self::packages).
    #[must_use]
    pub fn scan_packages(mut self, dir: impl AsRef<Path>) -> Self {
        self.sources.push(Source::ScanPackages {
            dir: dir.as_ref().to_path_buf(),
            group: DEFAULT_GROUP.to_owned(),
        });
        self
    }

    /// One more workspace `cdylib` package built alongside the programs and
    /// given a path constant (its uppercased crate name) but no
    /// `foreach_<group>!` arm — a driver guest the suite runs against every
    /// program rather than a fixture under test.
    #[must_use]
    pub fn extra_package(mut self, name: impl Into<String>) -> Self {
        self.extras.push(name.into());
        self
    }

    /// Rewrites the `[[example]]` stanzas after [`EXAMPLES_MARKER`] in
    /// `manifest` (relative to the workspace root) to match the programs, so
    /// adding a source file is enough.
    #[must_use]
    pub fn sync_examples(mut self, manifest: impl AsRef<Path>) -> Self {
        self.sync = Some(manifest.as_ref().to_path_buf());
        self
    }

    /// Names the group of the source added just before it — an
    /// [`examples`](Self::examples), [`packages`](Self::packages), or
    /// [`scan_packages`](Self::scan_packages) call; a scanned tree takes its
    /// groups from its directories.
    ///
    /// # Panics
    ///
    /// Panics when no source precedes it, or the preceding one is a
    /// [`scan`](Self::scan).
    #[must_use]
    pub fn group(mut self, name: impl Into<String>) -> Self {
        match self.sources.last_mut() {
            Some(
                Source::Examples { group, .. }
                | Source::Packages { group, .. }
                | Source::ScanPackages { group, .. },
            ) => *group = name.into(),
            Some(Source::Scan(_)) => {
                panic!("a scanned tree is grouped by its directories; `group` cannot rename it")
            }
            None => panic!("`group` names the source added before it; add one first"),
        }
        self
    }

    /// Extra `rerun-if-changed` paths relative to the workspace root, for
    /// inputs the nested build's dep-info misses (WIT read by proc macros,
    /// `Cargo.lock`).
    #[must_use]
    pub fn track<P: AsRef<Path>>(mut self, paths: impl IntoIterator<Item = P>) -> Self {
        self.tracked.extend(paths.into_iter().map(|path| path.as_ref().to_path_buf()));
        self
    }

    /// Runs the nested build and registers every input with cargo.
    ///
    /// A no-op producing an empty [`Built`] when the outer target is itself
    /// `wasm32`.
    ///
    /// # Panics
    ///
    /// Panics when a cargo build-script variable is missing, discovery finds
    /// an unexpected layout, the manifest to sync lacks the marker or no
    /// example source feeds it, the nested build fails, or an artifact is
    /// missing afterwards.
    #[must_use]
    pub fn build(self) -> Built {
        let header = format!(
            "// Generated by {}/build.rs through omnia_test::build.\n",
            std::env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "omnia-test".to_owned())
        );
        if env::outer_target_is_wasm32() {
            return Built {
                header,
                programs: Vec::new(),
                extras: Vec::new(),
                artifacts_dir: PathBuf::new(),
            };
        }

        let root = env::path_var("CARGO_MANIFEST_DIR").join(&self.root);
        let root = root
            .canonicalize()
            .unwrap_or_else(|err| panic!("resolving workspace root {}: {err}", root.display()));
        for tracked in &self.tracked {
            rerun_if_changed(&root.join(tracked));
        }

        let programs = self.programs(&root);
        if let Some(manifest) = &self.sync {
            assert!(
                self.sources.iter().any(Source::is_examples),
                "sync_examples refreshes an [[example]] list; packages have none"
            );
            let examples: Vec<Program> =
                programs.iter().filter(|program| program.example).cloned().collect();
            sync(&root.join(manifest), &examples);
        }

        let target_dir = env::path_var("OUT_DIR").join("fixtures");
        for mut command in self.nested_builds(&root, &target_dir, &programs) {
            let status = command
                .status()
                .unwrap_or_else(|err| panic!("spawning the {WASM_TARGET} fixture build: {err}"));
            assert!(
                status.success(),
                "the fixture components could not be built; install the target with `rustup \
                 target add {WASM_TARGET}` and retry"
            );
        }

        // Always `debug`: the nested build uses the dev profile regardless of
        // the outer profile.
        let built = Built {
            header,
            programs,
            extras: self.extras.iter().map(|package| extra_program(package)).collect(),
            artifacts_dir: target_dir.join(WASM_TARGET).join("debug"),
        };
        built.register_inputs();
        built
    }

    /// The programs every source names, in source order, discovering and
    /// registering scanned directories.
    fn programs(&self, root: &Path) -> Vec<Program> {
        self.sources
            .iter()
            .flat_map(|source| match source {
                Source::Examples { names, group } => names
                    .iter()
                    .map(|name| Program {
                        name: name.clone(),
                        constant: name.to_uppercase(),
                        group: group.clone(),
                        source: format!("examples/{name}.rs"),
                        example: true,
                    })
                    .collect(),
                Source::Scan(dir) => {
                    let dir = root.join(dir);
                    rerun_if_changed(&dir);
                    scan(&dir)
                }
                Source::Packages { names, group } => package_programs(group, names.clone()),
                Source::ScanPackages { dir, group } => {
                    let dir = root.join(dir);
                    rerun_if_changed(&dir);
                    package_programs(group, scan_packages(&dir))
                }
            })
            .collect()
    }

    /// The nested cargo invocations: one per example source, selecting that
    /// source's targets of the package, then one for every `cdylib` package
    /// — the package sources' programs and the extras together.
    fn nested_builds(&self, root: &Path, target_dir: &Path, programs: &[Program]) -> Vec<Command> {
        let mut commands = Vec::new();
        for source in &self.sources {
            let targets: Vec<&str> = match source {
                Source::Examples { names, .. } => {
                    names.iter().flat_map(|name| ["--example", name.as_str()]).collect()
                }
                Source::Scan(_) => vec!["--examples"],
                Source::Packages { .. } | Source::ScanPackages { .. } => continue,
            };
            let mut command = env::nested_build(root, target_dir);
            if let Some(package) = &self.package {
                command.arg(format!("--package={package}"));
            }
            command.args(targets);
            commands.push(command);
        }

        let packages: Vec<&str> = programs
            .iter()
            .filter(|program| !program.example)
            .map(|program| program.source.as_str())
            .chain(self.extras.iter().map(String::as_str))
            .collect();
        if !packages.is_empty() {
            let mut command = env::nested_build(root, target_dir);
            for package in packages {
                command.args(["--package", package]);
            }
            commands.push(command);
        }
        commands
    }
}

/// Split point for a synced `[[example]]` list: everything after this line
/// in the manifest is regenerated.
pub const EXAMPLES_MARKER: &str = "# Generated by build.rs from programs/<group>/<scenario>.rs.\n";

/// The outcome of [`Components::build`]: the programs and where their
/// artifacts landed.
#[derive(Clone, Debug)]
pub struct Built {
    header: String,
    programs: Vec<Program>,
    // `extra_package` components: a constant each, no group.
    extras: Vec<Program>,
    // The profile directory; examples are uplifted one level below it.
    artifacts_dir: PathBuf,
}

impl Built {
    /// The programs built, each source's sorted by name in source order;
    /// empty under a `wasm32` outer target.
    #[must_use]
    pub fn programs(&self) -> &[Program] {
        &self.programs
    }

    /// The extra packages built beside the programs (see
    /// [`Components::extra_package`]); empty under a `wasm32` outer target.
    #[must_use]
    pub fn extras(&self) -> &[Program] {
        &self.extras
    }

    /// The compiled component for `program`.
    #[must_use]
    pub fn artifact(&self, program: &Program) -> PathBuf {
        let file = format!("{}.wasm", program.name);
        if program.example {
            self.artifacts_dir.join("examples").join(file)
        } else {
            self.artifacts_dir.join(file)
        }
    }

    /// Asserts every artifact exists and registers each one's dep-info
    /// prerequisites with cargo.
    fn register_inputs(&self) {
        let mut seen = BTreeSet::new();
        for program in self.programs.iter().chain(&self.extras) {
            let artifact = self.artifact(program);
            assert!(
                artifact.exists(),
                "no artifact for `{}` at {}: is it {}?",
                program.name,
                artifact.display(),
                if program.example {
                    "an `[[example]]` of the package"
                } else {
                    "a `cdylib` package of the workspace"
                }
            );
            // Cargo's extra dep-info sits next to the uplifted artifact
            // (`{name}.d`) and lists the program's sources plus every local
            // path dependency, so edits there rebuild the fixture.
            let dep_info = artifact.with_extension("d");
            let contents = fs::read_to_string(&dep_info)
                .unwrap_or_else(|err| panic!("reading {}: {err}", dep_info.display()));
            for path in dep_info_paths(&contents) {
                if seen.insert(path.clone()) {
                    println!("cargo:rerun-if-changed={path}");
                }
            }
        }
    }

    /// Writes `gen.rs` (or another `file` name) under `OUT_DIR`: one
    /// `pub const <NAME>: &str` per program and extra package, and one
    /// `foreach_<group>!` per group, for the consumer to `include!`.
    ///
    /// # Panics
    ///
    /// Panics when `OUT_DIR` is unset, an artifact path is not UTF-8, or the
    /// file cannot be written.
    pub fn write_gen(&self, file: impl AsRef<Path>) {
        let artifacts: Vec<_> = self
            .programs
            .iter()
            .chain(&self.extras)
            .map(|program| {
                let path = self.artifact(program);
                let path = path.to_str().expect("artifact path is UTF-8").to_owned();
                Artifact { program, path }
            })
            .collect();
        let out = env::path_var("OUT_DIR").join(file);
        fs::write(&out, render_gen(&self.header, &artifacts))
            .unwrap_or_else(|err| panic!("writing {}: {err}", out.display()));
    }
}

fn rerun_if_changed(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
}

/// Discovers `<dir>/<group>/<scenario>.rs`, registering each directory and
/// file with cargo.
fn scan(dir: &Path) -> Vec<Program> {
    let mut groups = Vec::new();
    for entry in read_dir(dir) {
        let path = entry.path();
        let file_type = entry.file_type().expect("reading file type");
        assert!(
            !file_type.is_file(),
            "`{}` entries must be group directories, found `{}`",
            dir.display(),
            path.display()
        );
        if !file_type.is_dir() {
            continue;
        }
        rerun_if_changed(&path);
        let group = entry.file_name().into_string().expect("UTF-8 group directory");
        let mut scenarios = Vec::new();
        for file in read_dir(&path) {
            let file_path = file.path();
            assert!(
                !file.file_type().expect("reading scenario file type").is_dir(),
                "nested scenario groups are not supported: `{}`",
                file_path.display()
            );
            if file_path.extension().is_some_and(|extension| extension == "rs") {
                rerun_if_changed(&file_path);
                scenarios.push(
                    file_path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .expect("UTF-8 scenario stem")
                        .to_owned(),
                );
            }
        }
        groups.push((group, scenarios));
    }
    let relative = dir.file_name().map_or_else(String::new, |name| name.to_string_lossy().into());
    programs_from(&relative, groups)
}

/// Discovers `<dir>/<name>/Cargo.toml` package directories, registering the
/// parent with cargo; anything else under `dir` is a layout error.
fn scan_packages(dir: &Path) -> Vec<String> {
    let mut packages = Vec::new();
    for entry in read_dir(dir) {
        let path = entry.path();
        assert!(
            path.join("Cargo.toml").is_file(),
            "`{}` is not a crate directory (`{}` holds crate directories only)",
            path.display(),
            dir.display()
        );
        packages.push(entry.file_name().into_string().expect("UTF-8 crate directory"));
    }
    assert!(!packages.is_empty(), "no crate directories under {}", dir.display());
    packages
}

fn read_dir(dir: &Path) -> impl Iterator<Item = fs::DirEntry> {
    fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("reading {}: {err}", dir.display()))
        .map(|entry| entry.expect("reading directory entry"))
}

/// Names `<group>_<scenario>` programs from a `(group, scenarios)` listing,
/// with sources under `<programs_dir>/<group>/<scenario>.rs`, sorted by name.
fn programs_from(
    programs_dir: &str, groups: impl IntoIterator<Item = (String, Vec<String>)>,
) -> Vec<Program> {
    let mut programs: Vec<Program> = groups
        .into_iter()
        .flat_map(|(group, scenarios)| {
            scenarios.into_iter().map(move |scenario| {
                let name = format!("{group}_{scenario}");
                Program {
                    constant: name.to_uppercase(),
                    name,
                    source: format!("{programs_dir}/{group}/{scenario}.rs"),
                    group: group.clone(),
                    example: true,
                }
            })
        })
        .collect();
    programs.sort_by(|left, right| left.name.cmp(&right.name));
    programs
}

/// An ungrouped program for an extra `cdylib` package: a constant, no arm.
fn extra_program(package: &str) -> Program {
    let name = package.replace('-', "_");
    Program {
        constant: name.to_uppercase(),
        name,
        group: String::new(),
        source: package.to_owned(),
        example: false,
    }
}

/// One program per `cdylib` package in `group`, sorted by name: the crate
/// name (`-` as `_`) is the artifact stem and `foreach_<group>!` arm, the
/// package name is what the nested build is told, and the constant carries
/// the group so it reads beside scanned programs (`SOURCE_INTENT`).
fn package_programs(group: &str, packages: Vec<String>) -> Vec<Program> {
    let mut programs: Vec<Program> = packages
        .into_iter()
        .map(|package| {
            let name = package.replace('-', "_");
            Program {
                constant: format!("{group}_{name}").to_uppercase(),
                name,
                group: group.to_owned(),
                source: package,
                example: false,
            }
        })
        .collect();
    programs.sort_by(|left, right| left.name.cmp(&right.name));
    programs
}

/// Rewrites the `[[example]]` list in `manifest` when it disagrees with
/// `programs`.
fn sync(manifest: &Path, programs: &[Program]) {
    let current = fs::read_to_string(manifest)
        .unwrap_or_else(|err| panic!("reading {}: {err}", manifest.display()));
    let Some(next) = refresh_examples(&current, EXAMPLES_MARKER, programs) else {
        panic!(
            "{} must contain the line `{EXAMPLES_MARKER}` so the generated [[example]] list can \
             be refreshed",
            manifest.display()
        );
    };
    if current != next {
        fs::write(manifest, next)
            .unwrap_or_else(|err| panic!("writing {}: {err}", manifest.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanned_programs() {
        let programs = programs_from(
            "programs",
            [
                ("model".to_owned(), vec!["tools".to_owned(), "echo_text".to_owned()]),
                ("link".to_owned(), vec!["echo".to_owned()]),
            ],
        );
        assert_eq!(
            programs,
            [
                Program {
                    name: "link_echo".into(),
                    constant: "LINK_ECHO".into(),
                    group: "link".into(),
                    source: "programs/link/echo.rs".into(),
                    example: true,
                },
                Program {
                    name: "model_echo_text".into(),
                    constant: "MODEL_ECHO_TEXT".into(),
                    group: "model".into(),
                    source: "programs/model/echo_text.rs".into(),
                    example: true,
                },
                Program {
                    name: "model_tools".into(),
                    constant: "MODEL_TOOLS".into(),
                    group: "model".into(),
                    source: "programs/model/tools.rs".into(),
                    example: true,
                },
            ]
        );
    }

    // A package's crate name is the artifact stem and macro arm, the group
    // prefixes only the constant, and the build is told the package name.
    #[test]
    fn package_constants() {
        let programs =
            package_programs("source", vec!["typescript".to_owned(), "gtfs-adapter".to_owned()]);
        assert_eq!(
            programs,
            [
                Program {
                    name: "gtfs_adapter".into(),
                    constant: "SOURCE_GTFS_ADAPTER".into(),
                    group: "source".into(),
                    source: "gtfs-adapter".into(),
                    example: false,
                },
                Program {
                    name: "typescript".into(),
                    constant: "SOURCE_TYPESCRIPT".into(),
                    group: "source".into(),
                    source: "typescript".into(),
                    example: false,
                },
            ]
        );
    }

    // Examples are uplifted under `debug/examples/`; packages and extras sit
    // in the profile directory itself.
    #[test]
    fn artifact_layout() {
        let program = |name: &str, group: &str, example: bool| Program {
            name: name.into(),
            constant: name.to_uppercase(),
            group: group.into(),
            source: name.into(),
            example,
        };
        let built = Built {
            header: String::new(),
            programs: vec![
                program("adapter", "examples", true),
                program("intent", "source", false),
            ],
            extras: vec![program("caller", "", false)],
            artifacts_dir: PathBuf::from("/out/debug"),
        };
        assert_eq!(
            built.artifact(&built.programs[0]),
            Path::new("/out/debug/examples/adapter.wasm")
        );
        assert_eq!(built.artifact(&built.programs[1]), Path::new("/out/debug/intent.wasm"));
        assert_eq!(built.artifact(&built.extras[0]), Path::new("/out/debug/caller.wasm"));
    }

    #[test]
    fn builder_config() {
        let components = Components::in_workspace("../..")
            .package("test-programs")
            .scan("crates/test-programs/programs")
            .sync_examples("crates/test-programs/Cargo.toml")
            .track(["wit", "Cargo.lock"]);
        assert_eq!(components.package.as_deref(), Some("test-programs"));
        assert_eq!(components.sources, [Source::Scan("crates/test-programs/programs".into())]);
        assert_eq!(components.sync.as_deref(), Some(Path::new("crates/test-programs/Cargo.toml")));
        assert_eq!(components.tracked, [PathBuf::from("wit"), PathBuf::from("Cargo.lock")]);

        let explicit = Components::in_workspace(".").examples(["adapter"]).group("mock");
        assert_eq!(
            explicit.sources,
            [Source::Examples {
                names: vec!["adapter".to_owned()],
                group: "mock".to_owned(),
            }]
        );

        let packages = Components::in_workspace("../..")
            .scan_packages("sources")
            .group("source")
            .extra_package("caller");
        assert_eq!(
            packages.sources,
            [Source::ScanPackages {
                dir: "sources".into(),
                group: "source".to_owned(),
            }]
        );
        assert!(!packages.sources[0].is_examples());
        assert_eq!(packages.extras, ["caller"]);
    }

    // One build draws from several sources: `group` names the one added just
    // before it, and each keeps its own.
    #[test]
    fn mixed_sources() {
        let components = Components::in_workspace("../..")
            .scan_packages("sources")
            .group("adapter")
            .package("test-programs")
            .scan("crates/test-programs/programs")
            .packages(["caller"]);
        assert_eq!(
            components.sources,
            [
                Source::ScanPackages {
                    dir: "sources".into(),
                    group: "adapter".to_owned(),
                },
                Source::Scan("crates/test-programs/programs".into()),
                Source::Packages {
                    names: vec!["caller".to_owned()],
                    group: DEFAULT_GROUP.to_owned(),
                },
            ]
        );
    }

    #[test]
    #[should_panic(expected = "grouped by its directories")]
    fn group_after_scan() {
        let _ = Components::in_workspace(".").scan("programs").group("model");
    }

    #[test]
    #[should_panic(expected = "add one first")]
    fn group_before_source() {
        let _ = Components::in_workspace(".").group("model");
    }
}
