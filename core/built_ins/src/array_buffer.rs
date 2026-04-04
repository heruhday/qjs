use std::collections::HashMap;

use value::{JSValue, make_bool, make_number, make_undefined, to_f64};

use crate::{BuiltinHost, BuiltinMethod, create_builtin_callable, install_global_function};

const ARRAY_BUFFER_STATIC_METHODS: &[BuiltinMethod] = &[BuiltinMethod::new(
    "isView",
    "__builtin_array_buffer_is_view",
)];

const ARRAY_BUFFER_INSTANCE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("slice", "__builtin_array_buffer_slice"),
    BuiltinMethod::new("resize", "__builtin_array_buffer_resize"),
];

#[cfg(test)]
const ARRAY_BUFFER_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("isView", "__builtin_array_buffer_is_view"),
    BuiltinMethod::new("slice", "__builtin_array_buffer_slice"),
    BuiltinMethod::new("byteLength", "__builtin_array_buffer_get_byte_length"),
    BuiltinMethod::new("detached", "__builtin_array_buffer_get_detached"),
    BuiltinMethod::new(
        "maxByteLength",
        "__builtin_array_buffer_get_max_byte_length",
    ),
    BuiltinMethod::new("resizable", "__builtin_array_buffer_get_resizable"),
    BuiltinMethod::new("resize", "__builtin_array_buffer_resize"),
];

const KIND_PROP: &str = "__qjs_builtin_kind";
const DATA_PROP: &str = "__qjs_array_buffer_data";
const DETACHED_PROP: &str = "__qjs_array_buffer_detached";
const MAX_BYTE_LENGTH_PROP: &str = "__qjs_array_buffer_max_byte_length";
const BYTE_LENGTH_PROP: &str = "byteLength";
const MAX_BYTE_LENGTH_PUBLIC_PROP: &str = "maxByteLength";
const RESIZABLE_PUBLIC_PROP: &str = "resizable";
const DETACHED_PUBLIC_PROP: &str = "detached";

fn relative_index<H: BuiltinHost>(
    host: &mut H,
    value: Option<JSValue>,
    len: usize,
    default: usize,
) -> usize {
    let Some(value) = value else {
        return default;
    };

    let integer = to_f64(host.number_value(value))
        .filter(|number| number.is_finite())
        .map(f64::trunc)
        .unwrap_or(0.0);
    if integer.is_sign_negative() {
        ((len as f64) + integer).max(0.0) as usize
    } else {
        integer.min(len as f64) as usize
    }
}

fn requested_max_byte_length<H: BuiltinHost>(
    host: &mut H,
    value: Option<JSValue>,
) -> Option<usize> {
    let options = value.filter(|value| host.is_object(*value))?;
    let max = host.get_property(options, MAX_BYTE_LENGTH_PUBLIC_PROP);
    (!max.is_undefined()).then(|| {
        to_f64(host.number_value(max))
            .filter(|number| number.is_finite() && *number >= 0.0)
            .map(|number| number.trunc() as usize)
            .unwrap_or(0)
    })
}

fn attach_array_buffer_methods<H: BuiltinHost>(host: &mut H, target: JSValue) {
    for method in ARRAY_BUFFER_INSTANCE_METHODS {
        let function = create_builtin_callable(host, method.native_name);
        host.set_property(target, method.property_name, function);
    }
}

fn array_buffer_byte_length<H: BuiltinHost>(host: &H, buffer: JSValue) -> usize {
    host.bytes_from_value(host.get_property(buffer, DATA_PROP))
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn has_max_byte_length<H: BuiltinHost>(host: &H, buffer: JSValue) -> bool {
    host.has_property(buffer, MAX_BYTE_LENGTH_PROP)
}

fn sync_array_buffer_properties<H: BuiltinHost>(
    host: &mut H,
    buffer: JSValue,
    byte_length: usize,
    max_byte_length: Option<usize>,
    detached: bool,
) {
    host.set_property(buffer, BYTE_LENGTH_PROP, make_number(byte_length as f64));
    host.set_property(
        buffer,
        MAX_BYTE_LENGTH_PUBLIC_PROP,
        make_number(max_byte_length.unwrap_or(byte_length) as f64),
    );
    host.set_property(
        buffer,
        RESIZABLE_PUBLIC_PROP,
        make_bool(max_byte_length.is_some()),
    );
    host.set_property(buffer, DETACHED_PROP, make_bool(detached));
    host.set_property(buffer, DETACHED_PUBLIC_PROP, make_bool(detached));
}

fn create_array_buffer_instance<H: BuiltinHost>(
    host: &mut H,
    bytes: Vec<u8>,
    max_byte_length: Option<usize>,
) -> JSValue {
    let buffer = host.create_object();
    let kind = host.intern_string("ArrayBuffer");
    let data = host.bytes_to_value(&bytes);
    host.set_property(buffer, KIND_PROP, kind);
    host.set_property(buffer, DATA_PROP, data);
    if let Some(max_byte_length) = max_byte_length {
        host.set_property(
            buffer,
            MAX_BYTE_LENGTH_PROP,
            make_number(max_byte_length as f64),
        );
    }
    sync_array_buffer_properties(host, buffer, bytes.len(), max_byte_length, false);
    attach_array_buffer_methods(host, buffer);
    buffer
}

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(
        host,
        global_slots,
        "ArrayBuffer",
        "__builtin_array_buffer",
        ARRAY_BUFFER_STATIC_METHODS,
    );
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_array_buffer" => Some(array_buffer_constructor(host, args)),
        "__builtin_array_buffer_is_view" => {
            Some(make_bool(args.first().copied().is_some_and(|value| {
                matches!(
                    host.string_text(host.get_property(value, KIND_PROP)),
                    Some("DataView" | "TypedArray")
                )
            })))
        }
        "__builtin_array_buffer_slice" => Some(array_buffer_slice(host, this_value, args)),
        "__builtin_array_buffer_get_byte_length" => {
            Some(array_buffer_get_byte_length(host, this_value))
        }
        "__builtin_array_buffer_get_detached" => Some(array_buffer_get_detached(host, this_value)),
        "__builtin_array_buffer_get_max_byte_length" => {
            Some(array_buffer_get_max_byte_length(host, this_value))
        }
        "__builtin_array_buffer_get_resizable" => {
            Some(array_buffer_get_resizable(host, this_value))
        }
        "__builtin_array_buffer_resize" => Some(array_buffer_resize(host, this_value, args)),
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    (name == "__builtin_array_buffer").then(|| array_buffer_constructor(host, args))
}

pub(crate) fn is_array_buffer_instance<H: BuiltinHost>(host: &H, value: JSValue) -> bool {
    host.string_text(host.get_property(value, KIND_PROP)) == Some("ArrayBuffer")
}

pub(crate) fn array_buffer_data<H: BuiltinHost>(host: &H, value: JSValue) -> JSValue {
    host.get_property(value, DATA_PROP)
}

fn get_byte_length<H: BuiltinHost>(host: &mut H, buffer: JSValue) -> usize {
    array_buffer_byte_length(host, buffer)
}

fn array_buffer_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let length = args
        .first()
        .and_then(|&value| to_f64(host.number_value(value)))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.trunc() as usize)
        .unwrap_or(0);

    let max_byte_length =
        requested_max_byte_length(host, args.get(1).copied()).filter(|max| *max >= length);

    create_array_buffer_instance(host, vec![0; length], max_byte_length)
}

fn array_buffer_slice<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    if !is_array_buffer_instance(host, this_value) {
        return make_undefined();
    }

    let byte_length = get_byte_length(host, this_value);

    // Check if detached
    if host.is_truthy_value(host.get_property(this_value, DETACHED_PROP)) {
        return host.intern_string("TypeError: ArrayBuffer is detached");
    }

    let start_idx = relative_index(host, args.first().copied(), byte_length, 0);
    let end_idx = relative_index(host, args.get(1).copied(), byte_length, byte_length);

    let original_data = host.get_property(this_value, DATA_PROP);
    let data_bytes = host.bytes_from_value(original_data).unwrap_or_default();

    let sliced_data = if end_idx > start_idx && start_idx < data_bytes.len() {
        let end = end_idx.min(data_bytes.len());
        data_bytes[start_idx..end].to_vec()
    } else {
        Vec::new()
    };

    create_array_buffer_instance(host, sliced_data, None)
}

fn array_buffer_get_byte_length<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    if !is_array_buffer_instance(host, this_value) {
        return make_number(0.0);
    }

    // Check if detached
    if host.is_truthy_value(host.get_property(this_value, DETACHED_PROP)) {
        return make_number(0.0);
    }

    make_number(array_buffer_byte_length(host, this_value) as f64)
}

fn array_buffer_get_detached<H: BuiltinHost>(host: &H, this_value: JSValue) -> JSValue {
    if !is_array_buffer_instance(host, this_value) {
        return make_bool(false);
    }

    host.get_property(this_value, DETACHED_PROP)
}

fn array_buffer_get_max_byte_length<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    if !is_array_buffer_instance(host, this_value) {
        return make_number(0.0);
    }

    // Check if detached
    if host.is_truthy_value(host.get_property(this_value, DETACHED_PROP)) {
        return make_number(0.0);
    }

    if has_max_byte_length(host, this_value) {
        host.get_property(this_value, MAX_BYTE_LENGTH_PROP)
    } else {
        make_number(array_buffer_byte_length(host, this_value) as f64)
    }
}

fn array_buffer_get_resizable<H: BuiltinHost>(host: &H, this_value: JSValue) -> JSValue {
    if !is_array_buffer_instance(host, this_value) {
        return make_bool(false);
    }

    make_bool(has_max_byte_length(host, this_value))
}

fn array_buffer_resize<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    if !is_array_buffer_instance(host, this_value) {
        return host.intern_string("TypeError: not an ArrayBuffer");
    }

    // Check if detached
    if host.is_truthy_value(host.get_property(this_value, DETACHED_PROP)) {
        return host.intern_string("TypeError: Cannot resize a detached ArrayBuffer");
    }

    // Check if resizable
    if !has_max_byte_length(host, this_value) {
        return host.intern_string("TypeError: Cannot resize a fixed-length ArrayBuffer");
    }

    let new_length = args
        .first()
        .and_then(|&v| to_f64(host.number_value(v)))
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| v.trunc() as usize)
        .unwrap_or(0);

    let max_len = host.get_property(this_value, MAX_BYTE_LENGTH_PROP);

    // Get max byte length
    let max_byte_len = to_f64(host.number_value(max_len))
        .map(|v| v.trunc() as usize)
        .unwrap_or(0);

    if new_length > max_byte_len {
        return host.intern_string("RangeError: new length exceeds max byte length");
    }

    // Get current data
    let current_data = host.get_property(this_value, DATA_PROP);
    let mut bytes = host.bytes_from_value(current_data).unwrap_or_default();

    // Resize the buffer
    if new_length > bytes.len() {
        bytes.resize(new_length, 0);
    } else {
        bytes.truncate(new_length);
    }

    // Update the buffer
    let new_data = host.bytes_to_value(&bytes);
    host.set_property(this_value, DATA_PROP, new_data);
    sync_array_buffer_properties(host, this_value, new_length, Some(max_byte_len), false);

    make_undefined()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_array_buffer_methods_exist() {
        assert_eq!(ARRAY_BUFFER_METHODS.len(), 7);

        let method_names: Vec<&str> = ARRAY_BUFFER_METHODS
            .iter()
            .map(|m| m.property_name)
            .collect();
        assert!(method_names.contains(&"isView"));
        assert!(method_names.contains(&"slice"));
        assert!(method_names.contains(&"byteLength"));
        assert!(method_names.contains(&"detached"));
        assert!(method_names.contains(&"maxByteLength"));
        assert!(method_names.contains(&"resizable"));
        assert!(method_names.contains(&"resize"));
    }

    #[test]
    fn test_array_buffer_static_and_instance_method_sets() {
        let static_method_names: Vec<&str> = ARRAY_BUFFER_STATIC_METHODS
            .iter()
            .map(|m| m.property_name)
            .collect();
        let instance_method_names: Vec<&str> = ARRAY_BUFFER_INSTANCE_METHODS
            .iter()
            .map(|m| m.property_name)
            .collect();

        assert_eq!(static_method_names, vec!["isView"]);
        assert_eq!(instance_method_names, vec!["slice", "resize"]);
    }

    #[test]
    fn test_array_buffer_dispatch_names_are_unique() {
        let dispatch_names = [
            "__builtin_array_buffer",
            "__builtin_array_buffer_is_view",
            "__builtin_array_buffer_slice",
            "__builtin_array_buffer_get_byte_length",
            "__builtin_array_buffer_get_detached",
            "__builtin_array_buffer_get_max_byte_length",
            "__builtin_array_buffer_get_resizable",
            "__builtin_array_buffer_resize",
        ];

        let mut seen = HashSet::new();
        for name in dispatch_names {
            assert!(seen.insert(name), "Duplicate dispatch name: {name}");
        }
    }

    #[test]
    fn test_array_buffer_native_names_have_expected_prefix() {
        for method in ARRAY_BUFFER_METHODS {
            assert!(
                method.native_name.starts_with("__builtin_array_buffer"),
                "Invalid native name format: {}",
                method.native_name
            );
            assert!(
                !method.property_name.is_empty(),
                "Empty property name for native: {}",
                method.native_name
            );
        }
    }

    #[test]
    fn test_array_buffer_constructor_and_methods_are_present_in_registry() {
        let expected = [
            "isView",
            "slice",
            "byteLength",
            "detached",
            "maxByteLength",
            "resizable",
            "resize",
        ];

        for method in expected {
            assert!(
                ARRAY_BUFFER_METHODS
                    .iter()
                    .any(|entry| entry.property_name == method),
                "Method {method} not found in ARRAY_BUFFER_METHODS"
            );
        }
    }
}
