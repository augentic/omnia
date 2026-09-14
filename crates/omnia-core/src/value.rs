//! Guards on the plain component values that cross the link seam.

use wasmtime::component::{Type, Val, types};

/// Recursively reports whether a value carries a store-bound handle (a
/// resource, future, stream, or error-context).
#[must_use]
pub fn contains_handle(value: &Val) -> bool {
    handle_kind(value).is_some()
}

/// Names the kind of the first store-bound handle a value carries, if any.
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

/// Checks that every parameter and result type of `func` is a plain value.
///
/// # Errors
///
/// Returns the kind (`resource`, `future`, `stream`, or `error-context`) of the
/// first store-bound handle type the signature carries.
pub fn plain_signature(func: &types::ComponentFunc) -> Result<(), &'static str> {
    func.params().map(|(_, ty)| ty).chain(func.results()).try_for_each(|ty| plain_type(&ty))
}

fn plain_type(ty: &Type) -> Result<(), &'static str> {
    match ty {
        Type::Own(_) | Type::Borrow(_) => Err("resource"),
        Type::Future(_) => Err("future"),
        Type::Stream(_) => Err("stream"),
        Type::ErrorContext => Err("error-context"),
        Type::List(list) => plain_type(&list.ty()),
        Type::FixedLengthList(list) => plain_type(&list.ty()),
        Type::Map(map) => plain_type(&map.key()).and_then(|()| plain_type(&map.value())),
        Type::Record(record) => record.fields().try_for_each(|field| plain_type(&field.ty)),
        Type::Tuple(tuple) => tuple.types().try_for_each(|ty| plain_type(&ty)),
        Type::Variant(variant) => {
            variant.cases().try_for_each(|case| case.ty.as_ref().map_or(Ok(()), plain_type))
        }
        Type::Option(option) => plain_type(&option.ty()),
        Type::Result(result) => result
            .ok()
            .as_ref()
            .map_or(Ok(()), plain_type)
            .and_then(|()| result.err().as_ref().map_or(Ok(()), plain_type)),
        Type::Bool
        | Type::S8
        | Type::U8
        | Type::S16
        | Type::U16
        | Type::S32
        | Type::U32
        | Type::S64
        | Type::U64
        | Type::Float32
        | Type::Float64
        | Type::Char
        | Type::String
        | Type::Enum(_)
        | Type::Flags(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use wasmtime::component::types::ComponentItem;
    use wasmtime::component::{Component, types};
    use wasmtime::{Config, Engine};

    use super::plain_signature;

    fn signatures() -> (Engine, Component) {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.wasm_component_model_async(true);
        let engine = Engine::new(&config).expect("engine");
        let component = Component::new(
            &engine,
            r#"
            (component
              (import "sigs" (instance
                (export "r" (type $r (sub resource)))
                (type $rec-def (record (field "n" u32) (field "r" (own $r))))
                (export "rec" (type $rec (eq $rec-def)))
                (export "plain" (func (param "n" u32) (param "s" string) (result (list u8))))
                (export "own" (func (param "r" (own $r))))
                (export "borrow" (func (param "r" (borrow $r))))
                (export "future" (func (result (future u32))))
                (export "stream" (func (param "bytes" (stream u8))))
                (export "nested" (func (param "rec" (option $rec))))
              ))
            )
            "#,
        )
        .expect("type-only component");
        (engine, component)
    }

    fn signature(engine: &Engine, component: &Component, name: &str) -> types::ComponentFunc {
        let ComponentItem::ComponentInstance(sigs) =
            component.component_type().get_import(engine, "sigs").expect("sigs import").ty
        else {
            panic!("sigs import is not an instance");
        };
        let (_, types::ComponentExtern { ty, .. }) =
            sigs.exports(engine).find(|(func, _)| *func == name).expect(name);
        match ty {
            ComponentItem::ComponentFunc(func) => func,
            other => panic!("{name} export is {other:?}"),
        }
    }

    #[test]
    fn plain_params_and_results() {
        let (engine, component) = signatures();
        assert_eq!(plain_signature(&signature(&engine, &component, "plain")), Ok(()));
    }

    #[test]
    fn handle_kinds() {
        let (engine, component) = signatures();
        for (name, kind) in [
            ("own", "resource"),
            ("borrow", "resource"),
            ("future", "future"),
            ("stream", "stream"),
        ] {
            assert_eq!(plain_signature(&signature(&engine, &component, name)), Err(kind), "{name}");
        }
    }

    #[test]
    fn handle_nested_in_record() {
        let (engine, component) = signatures();
        assert_eq!(plain_signature(&signature(&engine, &component, "nested")), Err("resource"));
    }
}
