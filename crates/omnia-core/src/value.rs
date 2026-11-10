//! The value walk that keeps store-bound handles from crossing the link seam.

use wasmtime::component::Val;

/// Names the kind of the first store-bound handle a value carries, if any.
/// Recursive.
#[must_use]
pub fn handle_kind(value: &Val) -> Option<&'static str> {
    match value {
        Val::Resource(_) => Some("resource"),
        Val::Future(_) => Some("future"),
        Val::Stream(_) => Some("stream"),
        Val::ErrorContext(_) => Some("error-context"),
        Val::List(values) | Val::Tuple(values) | Val::FixedLengthList(values) => {
            values.iter().find_map(handle_kind)
        }
        Val::Map(entries) => {
            entries.iter().find_map(|(key, value)| handle_kind(key).or_else(|| handle_kind(value)))
        }
        Val::Record(fields) => fields.iter().find_map(|(_, value)| handle_kind(value)),
        Val::Variant(_, Some(value))
        | Val::Option(Some(value))
        | Val::Result(Ok(Some(value)) | Err(Some(value))) => handle_kind(value),
        _ => None,
    }
}
