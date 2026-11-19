//! The normal form of a [`Filter`]: the groups `omnia:docstore` takes.
//!
//! Every group must hold, and a group holds when any of its conditions does.

use super::{ComparisonOp, Filter, ScalarValue};
use crate::{Error, Result};

#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "wasi")),
    expect(dead_code, reason = "only the WASI glue reads a condition")
)]
#[derive(Clone)]
pub struct Condition {
    pub field: String,
    pub test: Test,
    pub negated: bool,
}

#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "wasi")),
    expect(dead_code, reason = "only the WASI glue reads a test")
)]
#[derive(Clone)]
pub enum Test {
    Eq(Value),
    Lt(Value),
    Lte(Value),
    Gt(Value),
    Gte(Value),
    In(Vec<Value>),
    StartsWith(String),
    EndsWith(String),
    Contains(String),
    Null,
}

#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "wasi")),
    expect(dead_code, reason = "only the WASI glue reads a value")
)]
#[derive(Clone)]
pub enum Value {
    Boolean(bool),
    Integer(i64),
    Number(f64),
    Text(String),
}

impl Filter {
    // `Filter::check` is this call, so no provider accepts a filter the WASI
    // glue cannot send
    pub(crate) fn groups(&self) -> Result<Vec<Vec<Condition>>> {
        let depth = self.depth();
        if depth > Self::MAX_DEPTH {
            let message =
                format!("the filter nests {depth} deep, past the limit of {}", Self::MAX_DEPTH);
            return Err(invalid(message));
        }
        Ok(Tree::of(self, false)?.groups()?.groups)
    }
}

// A filter with every `not` pushed down to a condition.
enum Tree {
    Leaf(Condition),
    And(Vec<Self>),
    Or(Vec<Self>),
}

impl Tree {
    // `negated` is whether an odd number of `not`s enclose `filter`, which
    // De Morgan's laws carry down to its conditions
    fn of(filter: &Filter, negated: bool) -> Result<Self> {
        match filter {
            Filter::And(children) | Filter::Or(children) if children.is_empty() => {
                Err(invalid("an `and` or `or` has no children"))
            }
            Filter::And(children) => {
                let children = Self::all(children, negated)?;
                Ok(if negated { Self::Or(children) } else { Self::And(children) })
            }
            Filter::Or(children) => {
                let children = Self::all(children, negated)?;
                Ok(if negated { Self::And(children) } else { Self::Or(children) })
            }
            Filter::Not(inner) => Self::of(inner, !negated),
            Filter::Compare { field, op, value } => Self::compare(field, *op, value, negated),
            Filter::InList { field, values } => Self::list(field, values, negated),
            Filter::NotInList { field, values } => Self::list(field, values, !negated),
            Filter::IsNull(field) => Ok(Self::leaf(field, Test::Null, negated)),
            Filter::IsNotNull(field) => Ok(Self::leaf(field, Test::Null, !negated)),
            Filter::Contains { field, pattern } => {
                Ok(Self::leaf(field, Test::Contains(pattern.clone()), negated))
            }
            Filter::StartsWith { field, pattern } => {
                Ok(Self::leaf(field, Test::StartsWith(pattern.clone()), negated))
            }
            Filter::EndsWith { field, pattern } => {
                Ok(Self::leaf(field, Test::EndsWith(pattern.clone()), negated))
            }
        }
    }

    fn all(children: &[Filter], negated: bool) -> Result<Vec<Self>> {
        children.iter().map(|child| Self::of(child, negated)).collect()
    }

    fn leaf(field: &str, test: Test, negated: bool) -> Self {
        Self::Leaf(Condition {
            field: field.to_owned(),
            test,
            negated,
        })
    }

    // `ne` is a negated `eq`, and an equality with `null` is the `null` test
    fn compare(field: &str, op: ComparisonOp, value: &ScalarValue, negated: bool) -> Result<Self> {
        let test = match op {
            ComparisonOp::Eq | ComparisonOp::Ne if matches!(value, ScalarValue::Null) => Test::Null,
            ComparisonOp::Eq | ComparisonOp::Ne => Test::Eq(Value::of(field, value)?),
            ComparisonOp::Lt => Test::Lt(Value::ordered(field, value)?),
            ComparisonOp::Lte => Test::Lte(Value::ordered(field, value)?),
            ComparisonOp::Gt => Test::Gt(Value::ordered(field, value)?),
            ComparisonOp::Gte => Test::Gte(Value::ordered(field, value)?),
        };
        Ok(Self::leaf(field, test, negated ^ (op == ComparisonOp::Ne)))
    }

    // an `in` takes no `null`, so a list that holds one is also a `null` test
    fn list(field: &str, values: &[ScalarValue], negated: bool) -> Result<Self> {
        if !(1..=Filter::MAX_LIST_LEN).contains(&values.len()) {
            let message = format!(
                "the list filter on `{field}` holds {} values, not 1 to {}",
                values.len(),
                Filter::MAX_LIST_LEN
            );
            return Err(invalid(message));
        }

        let is_null = |value: &&ScalarValue| matches!(value, ScalarValue::Null);
        let mut tests = Vec::with_capacity(2);
        if values.iter().any(|value| is_null(&value)) {
            tests.push(Self::leaf(field, Test::Null, negated));
        }
        let values = values
            .iter()
            .filter(|value| !is_null(value))
            .map(|value| Value::of(field, value))
            .collect::<Result<Vec<_>>>()?;
        if !values.is_empty() {
            tests.push(Self::leaf(field, Test::In(values), negated));
        }
        Ok(if negated { Self::And(tests) } else { Self::Or(tests) })
    }

    // distributes each `or` over the `and`s inside it
    fn groups(self) -> Result<Groups> {
        match self {
            Self::Leaf(condition) => Ok(Groups {
                groups: vec![vec![condition]],
                conditions: 1,
            }),
            Self::And(children) => children
                .into_iter()
                .try_fold(Groups::all(), |groups, child| groups.and(child.groups()?)),
            Self::Or(children) => children
                .into_iter()
                .try_fold(Groups::none(), |groups, child| groups.or(&child.groups()?)),
        }
    }
}

// An `and` of `or`s, and the conditions it holds.
struct Groups {
    groups: Vec<Vec<Condition>>,
    conditions: usize,
}

impl Groups {
    // the `and` of no groups, which every document passes
    const fn all() -> Self {
        Self {
            groups: Vec::new(),
            conditions: 0,
        }
    }

    // the `or` of no conditions, which no document passes
    fn none() -> Self {
        Self {
            groups: vec![Vec::new()],
            conditions: 0,
        }
    }

    fn and(mut self, other: Self) -> Result<Self> {
        let conditions = capped(self.conditions.saturating_add(other.conditions))?;
        self.groups.extend(other.groups);
        Ok(Self {
            groups: self.groups,
            conditions,
        })
    }

    // counts the distributed groups before it builds them
    fn or(self, other: &Self) -> Result<Self> {
        let conditions = capped(
            self.conditions
                .saturating_mul(other.groups.len())
                .saturating_add(other.conditions.saturating_mul(self.groups.len())),
        )?;
        let groups = self
            .groups
            .iter()
            .flat_map(|left| {
                other.groups.iter().map(move |right| left.iter().chain(right).cloned().collect())
            })
            .collect();
        Ok(Self { groups, conditions })
    }
}

fn capped(conditions: usize) -> Result<usize> {
    if conditions > Filter::MAX_CONDITIONS {
        let message = format!(
            "the filter holds more than {} conditions once written as an `and` of `or`s",
            Filter::MAX_CONDITIONS
        );
        return Err(invalid(message));
    }
    Ok(conditions)
}

impl Value {
    // JSON holds no bytes, and only a finite number
    fn of(field: &str, value: &ScalarValue) -> Result<Self> {
        match value {
            ScalarValue::Bool(boolean) => Ok(Self::Boolean(*boolean)),
            ScalarValue::Int32(integer) => Ok(Self::Integer(i64::from(*integer))),
            ScalarValue::Int64(integer) => Ok(Self::Integer(*integer)),
            ScalarValue::Float64(number) if number.is_finite() => Ok(Self::Number(*number)),
            ScalarValue::Float64(number) => {
                Err(invalid(format!("`{field}` is compared with {number}, which is not finite")))
            }
            ScalarValue::Str(text) | ScalarValue::Timestamp(text) => Ok(Self::Text(text.clone())),
            ScalarValue::Binary(_) => {
                Err(invalid(format!("`{field}` is compared with bytes, which JSON does not hold")))
            }
            ScalarValue::Null => {
                Err(invalid(format!("`{field}` is compared with `null` by a test that takes none")))
            }
        }
    }

    fn ordered(field: &str, value: &ScalarValue) -> Result<Self> {
        let unordered = match value {
            ScalarValue::Null => "`null`",
            ScalarValue::Bool(_) => "a boolean",
            value => return Self::of(field, value),
        };
        Err(invalid(format!("`{field}` is ordered by {unordered}, which has no order")))
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::server_error("invalid", message)
}
