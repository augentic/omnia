//! Command-line interface for omnia.

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context as _, Result, bail};
use clap::{ArgAction, Parser, Subcommand};

/// Command line interface for omnia.
#[derive(Parser, PartialEq, Eq)]
pub struct Cli {
    /// The command to execute.
    #[command(subcommand)]
    pub command: Command,

    /// Show more detail, once per level. Each `-v` raises the process
    /// tracing level one step from the mode's default: a command starts at
    /// `info`, a server at `warn`.
    #[arg(short, long, action = ArgAction::Count, global = true, conflicts_with = "quiet")]
    pub verbose: u8,

    /// Show less detail, once per level. Each `-q` lowers the process tracing
    /// level one step from the mode's default.
    #[arg(short, long, action = ArgAction::Count, global = true)]
    pub quiet: u8,
}

/// Subcommands for the omnia CLI.
#[derive(Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Run a guest (single-file shorthand) or a manifest-driven deployment.
    Run {
        /// The path to the wasm file to run. The file can either be a
        /// serialized (pre-compiled) wasmtime `Component` or standard
        /// WASI component. Optional when `--manifest` (or `OMNIA_MANIFEST`)
        /// names a deployment manifest instead.
        wasm: Option<PathBuf>,

        /// Path to a deployment manifest (`omnia.toml`) describing a multi-guest
        /// deployment. Falls back to the `OMNIA_MANIFEST` environment variable.
        #[arg(short, long)]
        manifest: Option<PathBuf>,

        /// Preopen a host directory into the guest sandbox (repeatable).
        /// Format: `path=<host-path>[,name=<guest-name>][,writable]`; `name`
        /// defaults to `.`. Layered on top of the manifest's mounts when
        /// `--manifest` is also given; a matching guest-visible name overrides
        /// the manifest mount (last-wins).
        #[arg(long = "mount")]
        mounts: Vec<MountArg>,

        /// Arguments forwarded to the guest as its argv (everything after
        /// `--`). Empty for a long-lived server; a `wasi:cli` command reads
        /// them as `wasi:cli/environment`'s `get-arguments`. `args[0]` is the
        /// program name, which the runtime core supplies.
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Compile the specified wasm32-wasip2 component.
    #[cfg(feature = "jit")]
    Compile {
        /// The path to the wasm file to compile.
        wasm: PathBuf,

        /// Where to write the artifact: a file path, or an existing directory
        /// the artifact lands in as `<input stem>.bin`. Without it, the
        /// artifact is written to stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// The target triple the artifact runs on; the host's when unset.
        #[arg(short, long)]
        target: Option<String>,
    },
}

/// A `--mount` argument: a host directory preopened under a guest-visible name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountArg {
    /// Guest-visible name `preopens.get-directories()` returns (e.g. `.`).
    pub name: String,
    /// Host path; a relative path resolves against the process working directory.
    pub host_path: PathBuf,
    /// Read+write when `true`; read-only otherwise.
    pub writable: bool,
}

impl FromStr for MountArg {
    type Err = anyhow::Error;

    /// Parse a CLI `--mount` spec: comma-separated `path=<host-path>`,
    /// `name=<guest-name>`, and a bare `writable` (or `writable=<bool>`) flag. A
    /// lone token without `=` is taken as the path, so `workspace` and
    /// `workspace,writable` are shorthands; `name` defaults to `.` and the mount
    /// is read-only unless `writable` is present.
    fn from_str(spec: &str) -> Result<Self> {
        let mut path: Option<PathBuf> = None;
        let mut name: Option<String> = None;
        let mut writable = false;

        for token in spec.split(',').map(str::trim).filter(|token| !token.is_empty()) {
            match token.split_once('=') {
                Some(("path", value)) => path = Some(PathBuf::from(value)),
                Some(("name", value)) => name = Some(value.to_owned()),
                Some(("writable", value)) => {
                    writable = value.parse().with_context(|| {
                        format!("mount `writable` expects a bool, got `{value}`")
                    })?;
                }
                Some((key, _)) => bail!("unknown mount key `{key}` in `--mount {spec}`"),
                None if token == "writable" => writable = true,
                None => {
                    if path.replace(PathBuf::from(token)).is_some() {
                        bail!("mount `--mount {spec}` sets the path more than once");
                    }
                }
            }
        }

        let path =
            path.with_context(|| format!("mount `--mount {spec}` is missing `path=<host-path>`"))?;
        Ok(Self {
            name: name.unwrap_or_else(|| ".".to_owned()),
            host_path: path,
            writable,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mount_full_spec() {
        let entry: MountArg = "path=workspace,name=.,writable".parse().expect("spec parses");
        assert_eq!(entry.host_path, PathBuf::from("workspace"));
        assert_eq!(entry.name, ".");
        assert!(entry.writable);
    }

    #[test]
    fn parse_mount_path() {
        let entry: MountArg = "workspace".parse().expect("bare path parses");
        assert_eq!(entry.host_path, PathBuf::from("workspace"));
        assert_eq!(entry.name, ".", "name defaults to `.`");
        assert!(!entry.writable, "a mount is read-only unless `writable` is given");
    }

    #[test]
    fn parse_mount_writable() {
        let entry: MountArg = "workspace,writable".parse().expect("shorthand parses");
        assert_eq!(entry.host_path, PathBuf::from("workspace"));
        assert!(entry.writable);
    }

    #[test]
    fn parse_mount_requires_path() {
        assert!("name=.,writable".parse::<MountArg>().is_err(), "a mount must name a path");
    }

    #[test]
    fn parse_mount_unknown() {
        assert!("path=x,bogus=1".parse::<MountArg>().is_err(), "unknown keys are rejected");
    }
}
