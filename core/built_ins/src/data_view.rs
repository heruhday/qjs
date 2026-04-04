use std::collections::HashMap;

use value::{JSValue, make_number, make_undefined, to_f64};

use crate::{BuiltinHost, BuiltinMethod, install_global_function, install_methods};

use super::array_buffer;

const DATA_VIEW_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("getUint8", "__builtin_data_view_get_uint8"),
    BuiltinMethod::new("setUint8", "__builtin_data_view_set_uint8"),
];

const KIND_PROP: &str = "__qjs_builtin_kind";
const BUFFER_PROP: &str = "__qjs_data_view_buffer";
const BYTE_OFFSET_PROP: &str = "__qjs_data_view_byte_offset";
const BYTE_LENGTH_PROP: &str = "__qjs_data_view_byte_length";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(host, global_slots, "DataView", "__builtin_data_view", &[]);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_data_view" => Some(data_view_constructor(host, args)),
        "__builtin_data_view_get_uint8" => Some(get_uint8(host, this_value, args)),
        "__builtin_data_view_set_uint8" => Some(set_uint8(host, this_value, args)),
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    (name == "__builtin_data_view").then(|| data_view_constructor(host, args))
}

fn data_view_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let buffer = match args.first().copied() {
        Some(value) if array_buffer::is_array_buffer_instance(host, value) => value,
        _ => array_buffer::construct(host, make_undefined(), "__builtin_array_buffer", &[])
            .unwrap_or_else(make_undefined),
    };

    let data = array_buffer::array_buffer_data(host, buffer);
    let total_length = host.array_values(data).map_or(0, |values| values.len());
    let byte_offset = args
        .get(1)
        .and_then(|&value| to_f64(host.number_value(value)))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.trunc() as usize)
        .unwrap_or(0)
        .min(total_length);
    let byte_length = args
        .get(2)
        .and_then(|&value| to_f64(host.number_value(value)))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.trunc() as usize)
        .unwrap_or(total_length.saturating_sub(byte_offset))
        .min(total_length.saturating_sub(byte_offset));

    let view = host.create_object();
    let kind = host.intern_string("DataView");
    host.set_property(view, KIND_PROP, kind);
    host.set_property(view, BUFFER_PROP, buffer);
    host.set_property(view, BYTE_OFFSET_PROP, make_number(byte_offset as f64));
    host.set_property(view, BYTE_LENGTH_PROP, make_number(byte_length as f64));
    host.set_property(view, "buffer", buffer);
    host.set_property(view, "byteOffset", make_number(byte_offset as f64));
    host.set_property(view, "byteLength", make_number(byte_length as f64));
    install_methods(host, view, DATA_VIEW_METHODS);
    view
}

fn get_uint8<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if host.string_text(host.get_property(this_value, KIND_PROP)) != Some("DataView") {
        return make_undefined();
    }

    let index = args
        .first()
        .and_then(|&value| to_f64(host.number_value(value)))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.trunc() as usize)
        .unwrap_or(0);
    let byte_offset = to_f64(host.get_property(this_value, BYTE_OFFSET_PROP))
        .unwrap_or(0.0)
        .max(0.0) as usize;
    let byte_length = to_f64(host.get_property(this_value, BYTE_LENGTH_PROP))
        .unwrap_or(0.0)
        .max(0.0) as usize;
    if index >= byte_length {
        return make_undefined();
    }

    let buffer = host.get_property(this_value, BUFFER_PROP);
    let data = array_buffer::array_buffer_data(host, buffer);
    host.get_index(data, byte_offset + index)
}

fn set_uint8<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if host.string_text(host.get_property(this_value, KIND_PROP)) != Some("DataView") {
        return make_undefined();
    }

    let index = args
        .first()
        .and_then(|&value| to_f64(host.number_value(value)))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.trunc() as usize)
        .unwrap_or(0);
    let next = args
        .get(1)
        .and_then(|&value| to_f64(host.number_value(value)))
        .filter(|value| value.is_finite())
        .map(|value| (value.trunc() as i64).clamp(0, 255) as f64)
        .unwrap_or(0.0);
    let byte_offset = to_f64(host.get_property(this_value, BYTE_OFFSET_PROP))
        .unwrap_or(0.0)
        .max(0.0) as usize;
    let byte_length = to_f64(host.get_property(this_value, BYTE_LENGTH_PROP))
        .unwrap_or(0.0)
        .max(0.0) as usize;
    if index >= byte_length {
        return make_undefined();
    }

    let buffer = host.get_property(this_value, BUFFER_PROP);
    let data = array_buffer::array_buffer_data(host, buffer);
    host.set_index(data, byte_offset + index, make_number(next))
}
