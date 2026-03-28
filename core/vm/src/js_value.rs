use crate::runtime::Context;

pub mod json;
pub mod msgpack;
pub mod serde_bridge;
pub mod tag_offset_arena_buffer;
pub mod yaml;

pub use self::tag_offset_arena_buffer::ValueError;
pub use ::value::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonValueError {
    message: String,
}

impl JsonValueError {
    fn unsupported(type_name: &'static str) -> Self {
        Self {
            message: format!("{type_name} cannot be converted to JSON"),
        }
    }

    fn cyclic(type_name: &'static str) -> Self {
        Self {
            message: format!("cyclic {type_name} cannot be converted to JSON"),
        }
    }

    fn invalid_number(value: f64) -> Self {
        Self {
            message: format!("non-finite number {value} cannot be converted to JSON"),
        }
    }

    fn parse(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for JsonValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for JsonValueError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YamlValueError {
    message: String,
}

impl YamlValueError {
    fn unsupported(type_name: &'static str) -> Self {
        Self {
            message: format!("{type_name} cannot be converted to YAML"),
        }
    }

    fn cyclic(type_name: &'static str) -> Self {
        Self {
            message: format!("cyclic {type_name} cannot be converted to YAML"),
        }
    }

    fn invalid_number(value: f64) -> Self {
        Self {
            message: format!("non-finite number {value} cannot be converted to YAML"),
        }
    }

    fn parse(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for YamlValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for YamlValueError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MsgpackValueError {
    message: String,
}

impl MsgpackValueError {
    fn unsupported(type_name: &'static str) -> Self {
        Self {
            message: format!("{type_name} cannot be converted to MsgPack"),
        }
    }

    fn cyclic(type_name: &'static str) -> Self {
        Self {
            message: format!("cyclic {type_name} cannot be converted to MsgPack"),
        }
    }

    fn invalid_number(value: f64) -> Self {
        Self {
            message: format!("non-finite number {value} cannot be converted to MsgPack"),
        }
    }

    fn parse(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for MsgpackValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MsgpackValueError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerdeValueError {
    message: String,
}

impl SerdeValueError {
    fn unsupported(type_name: &'static str) -> Self {
        Self {
            message: format!("{type_name} cannot be converted through serde"),
        }
    }

    fn cyclic(type_name: &'static str) -> Self {
        Self {
            message: format!("cyclic {type_name} cannot be converted through serde"),
        }
    }

    fn invalid_number(value: f64) -> Self {
        Self {
            message: format!("non-finite number {value} cannot be converted through serde"),
        }
    }

    fn serde_json(error: impl ToString) -> Self {
        Self {
            message: error.to_string(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for SerdeValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SerdeValueError {}

pub fn from_json(ctx: &Context, json: &str) -> Result<JSValue, JsonValueError> {
    json::from_json(ctx, json)
}

pub fn from_serde<T: serde::Serialize>(
    ctx: &Context,
    value: &T,
) -> Result<JSValue, SerdeValueError> {
    serde_bridge::from_serde(ctx, value)
}

pub fn from_serde_json<T: serde::Serialize>(ctx: &Context, value: &T) -> JSValue {
    match serde_json::to_value(value) {
        Ok(value) => serde_bridge::from_serde_json(ctx, &value),
        Err(_) => JSValue::undefined(),
    }
}

pub fn to_json(ctx: &Context, value: JSValue) -> Result<String, JsonValueError> {
    json::to_json(ctx, value)
}

pub fn to_pretty_json(ctx: &Context, value: JSValue) -> Result<String, JsonValueError> {
    json::to_pretty_json(ctx, value)
}

pub fn to_serde<T: serde::de::DeserializeOwned>(
    ctx: &Context,
    value: JSValue,
) -> Result<T, SerdeValueError> {
    serde_bridge::to_serde(ctx, value)
}

pub fn to_serde_json(ctx: &Context, value: JSValue) -> Result<String, SerdeValueError> {
    let value = serde_bridge::to_serde_json(ctx, value)?;
    serde_json::to_string(&value).map_err(SerdeValueError::serde_json)
}

pub fn from_yaml(ctx: &Context, yaml: &str) -> Result<JSValue, YamlValueError> {
    yaml::from_yaml(ctx, yaml)
}

pub fn to_yaml(ctx: &Context, value: JSValue) -> Result<String, YamlValueError> {
    yaml::to_yaml(ctx, value)
}

pub fn from_msgpack(ctx: &Context, bytes: &[u8]) -> Result<JSValue, MsgpackValueError> {
    msgpack::from_msgpack(ctx, bytes)
}

pub fn to_msgpack(ctx: &Context, value: JSValue) -> Result<Vec<u8>, MsgpackValueError> {
    msgpack::to_msgpack(ctx, value)
}

pub fn from_arena_buffer(ctx: &Context, bytes: &[u8]) -> Result<JSValue, ValueError> {
    tag_offset_arena_buffer::from_arena_buffer(ctx, bytes)
}

pub fn to_arena_buffer(ctx: &Context, value: JSValue) -> Result<Vec<u8>, ValueError> {
    tag_offset_arena_buffer::to_arena_buffer(ctx, value)
}

#[inline(always)]
pub fn make_object(ptr: *mut crate::vm::JSObject) -> JSValue {
    make_heap(ptr.cast::<GCHeader>())
}

#[inline(always)]
pub fn make_string(ptr: *mut crate::vm::JSString) -> JSValue {
    make_heap(ptr.cast::<GCHeader>())
}

#[inline(always)]
pub fn object_from_value(value: JSValue) -> Option<*mut crate::vm::JSObject> {
    let ptr = value.as_heap_ptr()?;
    (unsafe { (*ptr).obj_type } == ObjType::Object).then_some(ptr as *mut crate::vm::JSObject)
}

#[inline(always)]
pub fn string_from_value(value: JSValue) -> Option<*mut crate::vm::JSString> {
    let ptr = value.as_heap_ptr()?;
    (unsafe { (*ptr).obj_type } == ObjType::String).then_some(ptr as *mut crate::vm::JSString)
}

#[inline(always)]
pub fn is_object(value: JSValue) -> bool {
    object_from_value(value).is_some()
}

#[inline(always)]
pub fn is_string(value: JSValue) -> bool {
    value.as_atom().is_some() || string_from_value(value).is_some()
}
