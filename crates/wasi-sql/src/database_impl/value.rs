use chrono::{DateTime, NaiveDate, NaiveTime};

use crate::host::generated::omnia::sql::types::Value as WitValue;
use crate::host::{Error, Result, Value};

// `none` binds as `Str(None)`, which every backend binds as `NULL` to a
// parameter of any type.
pub fn bind(params: Vec<Option<WitValue>>) -> Result<Vec<Value>> {
    params.into_iter().zip(1..).map(|(param, n)| bind_one(param, n)).collect()
}

fn bind_one(param: Option<WitValue>, n: usize) -> Result<Value> {
    let Some(value) = param else {
        return Ok(Value::Str(None));
    };
    Ok(match value {
        WitValue::Boolean(b) => Value::Boolean(Some(b)),
        WitValue::Int32(i) => Value::Int32(Some(i)),
        WitValue::Int64(i) => Value::Int64(Some(i)),
        WitValue::Float32(f) => Value::Float(Some(f)),
        WitValue::Float64(f) => Value::Double(Some(f)),
        WitValue::Text(s) => Value::Str(Some(s)),
        WitValue::Binary(b) => Value::Binary(Some(b)),
        WitValue::Date(s) => Value::Date(Some(Format::Date.check(s, n)?)),
        WitValue::Time(s) => Value::Time(Some(Format::Time.check(s, n)?)),
        WitValue::Timestamp(s) => Value::Timestamp(Some(Format::Timestamp.check(s, n)?)),
        WitValue::TimestampTz(s) => Value::Timestamp(Some(Format::TimestampTz.check(s, n)?)),
    })
}

// A `u64` past `i64::MAX` reads as its decimal text, and a timestamp as
// `timestamp-tz` when it carries an offset.
pub fn read(value: Value) -> Option<WitValue> {
    match value {
        Value::Int32(i) => i.map(WitValue::Int32),
        Value::Int64(i) => i.map(WitValue::Int64),
        Value::Uint32(u) => u.map(|u| WitValue::Int64(i64::from(u))),
        Value::Uint64(u) => u.map(|u| {
            i64::try_from(u).map_or_else(|_| WitValue::Text(u.to_string()), WitValue::Int64)
        }),
        Value::Float(f) => f.map(WitValue::Float32),
        Value::Double(f) => f.map(WitValue::Float64),
        Value::Str(s) => s.map(WitValue::Text),
        Value::Boolean(b) => b.map(WitValue::Boolean),
        Value::Date(s) => s.map(WitValue::Date),
        Value::Time(s) => s.map(WitValue::Time),
        Value::Timestamp(s) => s.map(|s| {
            if DateTime::parse_from_rfc3339(&s).is_ok() {
                WitValue::TimestampTz(s)
            } else {
                WitValue::Timestamp(s)
            }
        }),
        Value::Binary(b) => b.map(WitValue::Binary),
    }
}

#[derive(Clone, Copy)]
enum Format {
    Date,
    Time,
    Timestamp,
    TimestampTz,
}

impl Format {
    fn check(self, text: String, n: usize) -> Result<String> {
        if self.holds(&text) {
            return Ok(text);
        }
        let expected = match self {
            Self::Date => "a `YYYY-MM-DD` date",
            Self::Time => "an `HH:MM:SS` time",
            Self::Timestamp => "a `YYYY-MM-DD HH:MM:SS` timestamp",
            Self::TimestampTz => "an RFC 3339 timestamp",
        };
        Err(Error::Invalid(format!("parameter `${n}` is not {expected}")))
    }

    fn holds(self, text: &str) -> bool {
        match self {
            Self::Date => is_date(text),
            Self::Time => is_time(text),
            Self::Timestamp => {
                text.split_once(' ').is_some_and(|(date, time)| is_date(date) && is_time(time))
            }
            Self::TimestampTz => DateTime::parse_from_rfc3339(text).is_ok(),
        }
    }
}

// chrono checks the ranges, and the shape pins the digits its parser lets vary
fn is_date(text: &str) -> bool {
    shaped(text, "9999-99-99") && NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
}

fn is_time(text: &str) -> bool {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, "0"));
    shaped(whole, "99:99:99")
        && !fraction.is_empty()
        && fraction.bytes().all(|byte| byte.is_ascii_digit())
        && NaiveTime::parse_from_str(text, "%H:%M:%S%.f").is_ok()
}

// `9` in the layout stands for any ASCII digit.
fn shaped(text: &str, layout: &str) -> bool {
    text.len() == layout.len()
        && text
            .bytes()
            .zip(layout.bytes())
            .all(|(byte, slot)| if slot == b'9' { byte.is_ascii_digit() } else { byte == slot })
}
