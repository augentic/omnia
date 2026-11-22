//! The support [`FromEnv`](derive@crate::FromEnv) expands to, which nothing
//! else calls.
//!
//! Each fn loads one shape of field over its `base`: the field's value when a
//! load runs over a value, else `None`. A field that fails records why in the
//! [`Report`], so a load fails once, naming every problem.

use std::env::{self, VarError};
use std::fmt::{self, Display};

/// An options struct the derive loads over a value or from nothing.
pub trait Fields: Sized {
    /// Loads `Self` over `base` from `source`, or records in `report` why it cannot.
    fn load(base: Option<Self>, source: &Source<'_>, report: &mut Report) -> Option<Self>;
}

/// Where a load reads its variables.
pub enum Source<'a> {
    /// The process environment, where a value that is not Unicode is invalid.
    Process,
    /// A lookup answering a variable's value, or `None` when it is unset.
    Lookup(&'a dyn Fn(&str) -> Option<String>),
}

impl Source<'_> {
    fn get(&self, name: &str) -> Result<Option<String>, &'static str> {
        match self {
            Self::Process => match env::var(name) {
                Ok(value) => Ok(Some(value)),
                Err(VarError::NotPresent) => Ok(None),
                Err(VarError::NotUnicode(_)) => Err("not valid Unicode"),
            },
            Self::Lookup(lookup) => Ok(lookup(name)),
        }
    }
}

/// The problems one load meets, in field order.
#[derive(Default)]
pub struct Report {
    problems: Vec<Problem>,
}

impl Report {
    fn only_missing(&self) -> bool {
        self.problems.iter().all(|problem| matches!(problem, Problem::Missing(_)))
    }

    // a lone `validate` error passes through whole, keeping its chain
    fn into_error(self) -> Option<anyhow::Error> {
        let mut problems = self.problems;
        if problems.len() > 1 {
            let joined = problems.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ");
            return Some(anyhow::Error::msg(joined));
        }
        problems.pop().map(|problem| match problem {
            Problem::Check(err) => err,
            problem => anyhow::Error::msg(problem.to_string()),
        })
    }
}

// a value is never kept, so no error can print one
enum Problem {
    Missing(&'static str),
    Invalid(&'static str, String),
    Check(anyhow::Error),
}

impl Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(name) => write!(f, "`{name}` is required but unset"),
            Self::Invalid(name, reason) => write!(f, "`{name}` is invalid: {reason}"),
            Self::Check(err) => write!(f, "{err:#}"),
        }
    }
}

/// Loads `T` over `base`, reading each variable from `source`.
///
/// # Errors
///
/// Fails with one error naming every required variable that is unset and
/// every variable whose value does not parse, or with the error of a
/// struct's `validate`.
pub fn load<T: Fields>(base: Option<T>, source: &Source<'_>) -> anyhow::Result<T> {
    let mut report = Report::default();
    match (T::load(base, source, &mut report), report.into_error()) {
        (Some(value), None) => Ok(value),
        (_, Some(err)) => Err(err),
        (None, None) => Err(anyhow::anyhow!("a load failed without naming a problem")),
    }
}

/// Reads a field from `name`, falling back to `base`, else recording it as missing.
pub fn required<T, E: Display>(
    source: &Source<'_>, report: &mut Report, name: &'static str,
    parse: impl FnOnce(&str) -> Result<T, E>, base: Option<T>,
) -> Option<T> {
    let value = read(source, report, name, parse).ok()?.or(base);
    if value.is_none() {
        report.problems.push(Problem::Missing(name));
    }
    value
}

// `Err` once a set value fails, which `report` then holds
fn read<T, E: Display>(
    source: &Source<'_>, report: &mut Report, name: &'static str,
    parse: impl FnOnce(&str) -> Result<T, E>,
) -> Result<Option<T>, ()> {
    let reason = match source.get(name) {
        Ok(None) => return Ok(None),
        Ok(Some(value)) => match parse(&value) {
            Ok(value) => return Ok(Some(value)),
            Err(err) => err.to_string(),
        },
        Err(reason) => reason.to_owned(),
    };
    report.problems.push(Problem::Invalid(name, reason));
    Err(())
}

/// Reads a field from `name`, falling back to `base`, else to its `default` literal.
pub fn defaulted<T, E: Display>(
    source: &Source<'_>, report: &mut Report, name: &'static str,
    parse: impl Fn(&str) -> Result<T, E>, base: Option<T>, field: &str, literal: &str,
) -> Option<T> {
    let value = read(source, report, name, &parse).ok()?.or(base);
    Some(value.unwrap_or_else(|| default(field, literal, parse)))
}

/// Reads an `Option` field from `name`, falling back to `base`, else to `None`.
pub fn optional<T, E: Display>(
    source: &Source<'_>, report: &mut Report, name: &'static str,
    parse: impl FnOnce(&str) -> Result<T, E>, base: Option<Option<T>>,
) -> Option<Option<T>> {
    Some(read(source, report, name, parse).ok()?.or_else(|| base.flatten()))
}

/// Takes a field set in code alone from `base`, else from its `default` literal.
pub fn code<T, E: Display>(
    base: Option<T>, field: &str, literal: &str, parse: impl FnOnce(&str) -> Result<T, E>,
) -> Option<T> {
    Some(base.unwrap_or_else(|| default(field, literal, parse)))
}

/// Loads a `nested` `Option` field, `None` when every problem inside is an unset variable.
pub fn nested<T: Fields>(
    base: Option<Option<T>>, source: &Source<'_>, report: &mut Report,
) -> Option<Option<T>> {
    if let Some(Some(inner)) = base {
        return T::load(Some(inner), source, report).map(Some);
    }
    let mut inner = Report::default();
    match T::load(None, source, &mut inner) {
        Some(value) => Some(Some(value)),
        None if inner.only_missing() => Some(None),
        None => {
            report.problems.append(&mut inner.problems);
            None
        }
    }
}

/// Returns `value` once the struct's `validate` accepts it, else records its error.
pub fn check<T>(
    value: T, report: &mut Report, validate: impl FnOnce(&T) -> anyhow::Result<()>,
) -> Option<T> {
    match validate(&value) {
        Ok(()) => Some(value),
        Err(err) => {
            report.problems.push(Problem::Check(err));
            None
        }
    }
}

/// Parses a field's `default` literal.
///
/// # Panics
///
/// When `literal` does not parse, which is a bug in the deriving crate.
pub fn default<T, E: Display>(
    field: &str, literal: &str, parse: impl FnOnce(&str) -> Result<T, E>,
) -> T {
    match parse(literal) {
        Ok(value) => value,
        Err(err) => panic!("the default `{literal}` of `{field}` does not parse: {err}"),
    }
}
