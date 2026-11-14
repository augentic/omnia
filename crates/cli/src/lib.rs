#![doc = include_str!("../README.md")]

mod cli;
mod entry;

pub use clap::Parser;

pub use self::cli::{Cli, Command, MountArg};
pub use self::entry::{PlanError, RunPlan, RunSource, plan};
