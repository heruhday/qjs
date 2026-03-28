use std::collections::HashSet;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::gc::Gc;
use crate::heap::{QArray, QObject, QString};
use crate::runtime::Context;

use super::{HeapKind, SerdeValueError, Value};

pub(super) fn from_serde<T: Serialize>(ctx: &Context, value: &T) -> Result<Value, SerdeValueError> {
    let value = serde_json::to_value(value).map_err(SerdeValueError::serde_json)?;
    Ok(from_serde_json(ctx, &value))
}

pub(super) fn to_serde<T: DeserializeOwned>(
    ctx: &Context,
    value: Value,
) -> Result<T, SerdeValueError> {
    let value = to_serde_json(ctx, value)?;
    serde_json::from_value(value).map_err(SerdeValueError::serde_json)
}

pub(super) fn from_serde_json(ctx: &Context, value: &serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::null(),
        serde_json::Value::Bool(value) => Value::from(*value),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Value::from(value)
            } else if let Some(value) = value.as_u64() {
                Value::from(value)
            } else {
                Value::from(value.as_f64().expect("serde_json number must be finite"))
            }
        }
        serde_json::Value::String(value) => Value::from(ctx.intern(value)),
        serde_json::Value::Array(items) => {
            let array = ctx.new_array();
            {
                let mut array_ref = array.borrow_mut();
                for item in items {
                    array_ref.push(from_serde_json(ctx, item));
                }
            }
            Value::from(array)
        }
        serde_json::Value::Object(entries) => {
            let object = ctx.new_object();
            {
                let mut object_ref = object.borrow_mut();
                for (key, value) in entries {
                    object_ref.set(ctx.intern(key), from_serde_json(ctx, value));
                }
            }
            Value::from(object)
        }
    }
}

pub(super) fn to_serde_json(
    ctx: &Context,
    value: Value,
) -> Result<serde_json::Value, SerdeValueError> {
    let mut seen = HashSet::new();
    to_serde_json_impl(ctx, value, &mut seen)
}

#[inline]
fn to_serde_json_impl(
    ctx: &Context,
    value: Value,
    seen: &mut HashSet<usize>,
) -> Result<serde_json::Value, SerdeValueError> {
    if value.is_null() || value.is_undefined() {
        return Ok(serde_json::Value::Null);
    }

    if let Some(value) = value.as_bool() {
        return Ok(serde_json::Value::Bool(value));
    }

    if let Some(value) = value.as_i32() {
        return Ok(serde_json::Value::Number(serde_json::Number::from(value)));
    }

    if let Some(value) = value.as_f64() {
        let number = serde_json::Number::from_f64(value)
            .ok_or_else(|| SerdeValueError::invalid_number(value))?;
        return Ok(serde_json::Value::Number(number));
    }

    if let Some(atom) = value.as_atom() {
        return Ok(serde_json::Value::String(ctx.resolve(atom)));
    }

    let ptr = value
        .as_heap_ptr()
        .map(|ptr| ptr as usize)
        .ok_or_else(|| SerdeValueError::unsupported("unknown value"))?;
    if !seen.insert(ptr) {
        return Err(SerdeValueError::cyclic(value.type_name()));
    }

    let result = match value.heap_kind() {
        Some(HeapKind::Object) => {
            let object = Gc::<QObject>::try_from(value)
                .map_err(|_| SerdeValueError::unsupported("object"))?;
            let object_ref = object.borrow();
            let mut out = serde_json::Map::with_capacity(object_ref.shape.props.len());
            for (&atom, &index) in &object_ref.shape.props {
                let child = object_ref
                    .values
                    .get(index)
                    .copied()
                    .unwrap_or_else(Value::undefined);
                out.insert(ctx.resolve(atom), to_serde_json_impl(ctx, child, seen)?);
            }
            serde_json::Value::Object(out)
        }
        Some(HeapKind::Array) => {
            let array =
                Gc::<QArray>::try_from(value).map_err(|_| SerdeValueError::unsupported("array"))?;
            let array_ref = array.borrow();
            let mut out = Vec::with_capacity(array_ref.elements.len());
            for &child in &array_ref.elements {
                out.push(to_serde_json_impl(ctx, child, seen)?);
            }
            serde_json::Value::Array(out)
        }
        Some(HeapKind::String) => {
            let string = Gc::<QString>::try_from(value)
                .map_err(|_| SerdeValueError::unsupported("string object"))?;
            let text = ctx.resolve(string.borrow().atom);
            serde_json::Value::String(text)
        }
        Some(kind) => return Err(SerdeValueError::unsupported(kind.type_name())),
        None => return Err(SerdeValueError::unsupported("unknown value")),
    };

    seen.remove(&ptr);
    Ok(result)
}
