//! The verbosity flags: counting `-v`/`-q` in argv and stepping a run's
//! levels by them — the run's from its mode's default, the runtime's own
//! crates' from a server's.

use crate::{DeploymentBuilder, LevelFilter, Mode};

/// The levels a run's verbosity flags select, each `None` when they select
/// nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Levels {
    /// The run's level: the host console's bare level and every guest's.
    pub run: Option<LevelFilter>,
    /// The level of the runtime's own crates on the host console.
    pub runtime: Option<LevelFilter>,
}

impl Levels {
    /// Select the levels on `builder`, leaving one selecting nothing unset.
    pub const fn apply(self, builder: DeploymentBuilder) -> DeploymentBuilder {
        let builder = match self.run {
            Some(level) => builder.level(level),
            None => builder,
        };
        match self.runtime {
            Some(level) => builder.runtime_level(level),
            None => builder,
        }
    }
}

/// The levels `verbose` and `quiet` select for a run in `mode`.
///
/// The run steps from the [mode's default](Mode::level); the runtime's own
/// crates step from a server's whatever the mode, so a command's console
/// shows its guests' progress at `info` and the runtime's from `-v`, as a
/// server's does — see [`DeploymentBuilder::runtime_level`].
pub fn levels(mode: Mode, verbose: u8, quiet: u8) -> Levels {
    Levels {
        run: level(mode.level(), verbose, quiet),
        runtime: level(Mode::Server.level(), verbose, quiet),
    }
}

/// The level `verbose` and `quiet` counts select from `default`, or `None`
/// when they select nothing.
///
/// Each `-v` is one rung up the scale `off`, `error`, `warn`, `info`,
/// `debug`, `trace` and each `-q` one rung down, clamped at both ends. No
/// flag selects nothing, so the process `RUST_LOG` stands. Both flags at once
/// select nothing either: the guest's grammar refuses the pair with its own
/// usage error, and the host has no level to apply before it does.
pub fn level(default: LevelFilter, verbose: u8, quiet: u8) -> Option<LevelFilter> {
    match (verbose, quiet) {
        (0, 0) => None,
        (_, 0) | (0, _) => {
            let start = LADDER.iter().position(|&rung| rung == default)?;
            let rung = (start + usize::from(verbose)).saturating_sub(usize::from(quiet));
            Some(LADDER[rung.min(LADDER.len() - 1)])
        }
        _ => None,
    }
}

/// The `(verbose, quiet)` counts the verbosity flags in `args` carry.
///
/// Counts `--verbose`, `--quiet`, and the short clusters of one letter —
/// `-v`, `-vv`, `-q`, `-qq`, … — each letter once; every other token passes
/// unread, and nothing after a literal `--` is read. A cluster of mixed
/// letters is left to the guest's grammar, whose flags may take values.
pub fn scan(args: &[String]) -> (u8, u8) {
    let mut verbose = 0u8;
    let mut quiet = 0u8;
    for arg in args.iter().take_while(|arg| *arg != "--") {
        match arg.as_str() {
            "--verbose" => verbose = verbose.saturating_add(1),
            "--quiet" => quiet = quiet.saturating_add(1),
            _ => match arg.strip_prefix('-') {
                Some(letters) if cluster_of(letters, b'v') => {
                    verbose = verbose.saturating_add(count(letters));
                }
                Some(letters) if cluster_of(letters, b'q') => {
                    quiet = quiet.saturating_add(count(letters));
                }
                _ => {}
            },
        }
    }
    (verbose, quiet)
}

// Whether `letters` is one or more of `letter` and nothing else.
fn cluster_of(letters: &str, letter: u8) -> bool {
    !letters.is_empty() && letters.bytes().all(|b| b == letter)
}

fn count(letters: &str) -> u8 {
    u8::try_from(letters.len()).unwrap_or(u8::MAX)
}

// The scale the flags step along, quietest first.
const LADDER: [LevelFilter; 6] = [
    LevelFilter::OFF,
    LevelFilter::ERROR,
    LevelFilter::WARN,
    LevelFilter::INFO,
    LevelFilter::DEBUG,
    LevelFilter::TRACE,
];
