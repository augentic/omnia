use crate::{Condition, Error, Filter, Result, Test, Value};

// The most conditions a filter holds, and the most values one `in` holds.
const MAX_CONDITIONS: usize = 100;
const MAX_IN_VALUES: usize = 100;

// a filter the WIT refuses is `invalid` before the backend sees the query
pub fn check(filter: &Filter) -> Result<()> {
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
