use super::CollectionHandle;
use crate::host::generated::omnia::docstore::collection::{Condition, Filter, Test, Value};
use crate::host::{ComparisonOp, Error, FilterTree, Result, ScalarValue};

// The most conditions a filter holds, and the most values one `in` holds.
const MAX_CONDITIONS: usize = 100;
const MAX_IN_VALUES: usize = 100;

// The filter as the tree a backend evaluates, `None` when it is empty. A
// filter the WIT refuses is `invalid`, and one the tree cannot express is
// `unsupported`, both before the backend sees the query.
pub fn tree(collection: &CollectionHandle, filter: Filter) -> Result<Option<FilterTree>> {
    check(&filter)?;

    let mut groups = Vec::with_capacity(filter.len());
    for group in filter {
        let leaves = group
            .into_iter()
            .map(|condition| leaf(collection, condition))
            .collect::<Result<Vec<_>>>()?;
        groups.push(join(leaves, FilterTree::Or));
    }
    Ok((!groups.is_empty()).then(|| join(groups, FilterTree::And)))
}

fn check(filter: &Filter) -> Result<()> {
    if filter.iter().any(Vec::is_empty) {
        return Err(Error::Invalid("a filter group holds no condition".to_owned()));
    }
    let conditions: usize = filter.iter().map(Vec::len).sum();
    if conditions > MAX_CONDITIONS {
        return Err(Error::Invalid(format!(
            "the filter holds {conditions} conditions, past the limit of {MAX_CONDITIONS}"
        )));
    }
    filter.iter().flatten().try_for_each(check_condition)
}

fn check_condition(condition: &Condition) -> Result<()> {
    let field = &condition.field;
    match &condition.test {
        Test::Lt(Value::Boolean(_))
        | Test::Lte(Value::Boolean(_))
        | Test::Gt(Value::Boolean(_))
        | Test::Gte(Value::Boolean(_)) => {
            Err(Error::Invalid(format!("`{field}` is ordered by a boolean, which has no order")))
        }
        Test::In(values) if !(1..=MAX_IN_VALUES).contains(&values.len()) => {
            Err(Error::Invalid(format!(
                "the `in` on `{field}` holds {} values, not 1 to {MAX_IN_VALUES}",
                values.len()
            )))
        }
        Test::In(values) => values.iter().try_for_each(|value| check_value(field, value)),
        Test::Eq(value)
        | Test::Lt(value)
        | Test::Lte(value)
        | Test::Gt(value)
        | Test::Gte(value)
        | Test::ArrayContains(value) => check_value(field, value),
        Test::StartsWith(_) | Test::EndsWith(_) | Test::Contains(_) | Test::Null => Ok(()),
    }
}

fn check_value(field: &str, value: &Value) -> Result<()> {
    match value {
        Value::Number(number) if !number.is_finite() => {
            Err(Error::Invalid(format!("`{field}` is tested with {number}, which is not finite")))
        }
        _ => Ok(()),
    }
}

fn leaf(collection: &CollectionHandle, condition: Condition) -> Result<FilterTree> {
    let Condition { field, test, negated } = condition;
    let compare = |op, value| FilterTree::Compare {
        field: field.clone(),
        op,
        value: scalar(value),
    };

    let leaf = match test {
        Test::Eq(value) => compare(ComparisonOp::Eq, value),
        Test::Lt(value) => compare(ComparisonOp::Lt, value),
        Test::Lte(value) => compare(ComparisonOp::Lte, value),
        Test::Gt(value) => compare(ComparisonOp::Gt, value),
        Test::Gte(value) => compare(ComparisonOp::Gte, value),
        Test::In(values) => FilterTree::InList {
            field,
            values: values.into_iter().map(scalar).collect(),
        },
        Test::StartsWith(pattern) => FilterTree::StartsWith { field, pattern },
        Test::EndsWith(pattern) => FilterTree::EndsWith { field, pattern },
        Test::Contains(pattern) => FilterTree::Contains { field, pattern },
        Test::Null => FilterTree::IsNull(field),
        Test::ArrayContains(_) => {
            return Err(collection.unsupported(&format!("test the elements of `{field}`")));
        }
    };
    Ok(if negated { FilterTree::Not(Box::new(leaf)) } else { leaf })
}

// A backend that types a stored integer by its range, as Azure Table does,
// compares an integer that fits in 32 bits as one.
fn scalar(value: Value) -> ScalarValue {
    match value {
        Value::Boolean(boolean) => ScalarValue::Boolean(boolean),
        Value::Integer(integer) => {
            i32::try_from(integer).map_or(ScalarValue::Int64(integer), ScalarValue::Int32)
        }
        Value::Number(number) => ScalarValue::Float64(number),
        Value::Text(text) => ScalarValue::Str(text),
    }
}

// a lone node stands for itself, so a backend sees no one-child `and` or `or`
fn join(mut nodes: Vec<FilterTree>, node: fn(Vec<FilterTree>) -> FilterTree) -> FilterTree {
    if nodes.len() == 1 { nodes.remove(0) } else { node(nodes) }
}
