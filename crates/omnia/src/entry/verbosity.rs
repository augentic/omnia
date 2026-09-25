//! The verbosity flags: counting `-v`/`-q` in argv and stepping a mode's
//! default level by them.

use crate::LevelFilter;

// The scale the flags step along, quietest first.
const LADDER: [LevelFilter; 6] = [
    LevelFilter::OFF,
    LevelFilter::ERROR,
    LevelFilter::WARN,
    LevelFilter::INFO,
    LevelFilter::DEBUG,
    LevelFilter::TRACE,
];

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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|token| (*token).to_owned()).collect()
    }

    #[test]
    fn ladder_command() {
        assert_eq!(level(LevelFilter::INFO, 0, 2), Some(LevelFilter::ERROR));
        assert_eq!(level(LevelFilter::INFO, 0, 1), Some(LevelFilter::WARN));
        assert_eq!(level(LevelFilter::INFO, 0, 0), None);
        assert_eq!(level(LevelFilter::INFO, 1, 0), Some(LevelFilter::DEBUG));
        assert_eq!(level(LevelFilter::INFO, 2, 0), Some(LevelFilter::TRACE));
    }

    #[test]
    fn ladder_server() {
        assert_eq!(level(LevelFilter::WARN, 0, 2), Some(LevelFilter::OFF));
        assert_eq!(level(LevelFilter::WARN, 0, 1), Some(LevelFilter::ERROR));
        assert_eq!(level(LevelFilter::WARN, 0, 0), None);
        assert_eq!(level(LevelFilter::WARN, 1, 0), Some(LevelFilter::INFO));
        assert_eq!(level(LevelFilter::WARN, 2, 0), Some(LevelFilter::DEBUG));
        assert_eq!(level(LevelFilter::WARN, 3, 0), Some(LevelFilter::TRACE));
    }

    #[test]
    fn ladder_clamps() {
        assert_eq!(level(LevelFilter::INFO, 4, 0), Some(LevelFilter::TRACE));
        assert_eq!(level(LevelFilter::INFO, u8::MAX, 0), Some(LevelFilter::TRACE));
        assert_eq!(level(LevelFilter::INFO, 0, 4), Some(LevelFilter::OFF));
        assert_eq!(level(LevelFilter::INFO, 0, u8::MAX), Some(LevelFilter::OFF));
    }

    #[test]
    fn ladder_conflict() {
        assert_eq!(level(LevelFilter::INFO, 1, 1), None);
        assert_eq!(level(LevelFilter::INFO, 2, 1), None);
    }

    #[test]
    fn scan_counts() {
        assert_eq!(scan(&args(&["-vv"])), (2, 0));
        assert_eq!(scan(&args(&["-v", "show", "-v"])), (2, 0));
        assert_eq!(scan(&args(&["--verbose"])), (1, 0));
        assert_eq!(scan(&args(&["-qq"])), (0, 2));
        assert_eq!(scan(&args(&["--quiet", "-q"])), (0, 2));
        assert_eq!(scan(&args(&["-v", "-q"])), (1, 1));
        assert_eq!(scan(&args(&["show", "spec"])), (0, 0));
    }

    // A value-taking flag's argument, a mixed cluster, a bare `-`, and a
    // longer flag are the guest's: none of them counts.
    #[test]
    fn scan_leaves_other_tokens() {
        assert_eq!(scan(&args(&["-c", "-v", "-d", "v", "-vq", "-", "--verbosity"])), (1, 0));
    }

    #[test]
    fn scan_after_double_dash() {
        assert_eq!(scan(&args(&["-v", "--", "-vv", "--quiet"])), (1, 0));
    }
}
