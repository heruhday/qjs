use std::collections::HashMap;

use value::{JSValue, make_number, make_undefined, to_f64};

use crate::{
    BuiltinHost, BuiltinMethod, create_array_from_values, install_global_function, install_methods,
};

const TYPED_ARRAY_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("from", "__builtin_typed_array_from"),
    BuiltinMethod::new("of", "__builtin_typed_array_of"),
];

const INSTANCE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("at", "__builtin_typed_array_at"),
    BuiltinMethod::new("set", "__builtin_typed_array_set"),
    BuiltinMethod::new("toArray", "__builtin_typed_array_to_array"),
];

const KIND_PROP: &str = "__qjs_builtin_kind";
const DATA_PROP: &str = "__qjs_typed_array_data";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(
        host,
        global_slots,
        "TypedArray",
        "__builtin_typed_array",
        TYPED_ARRAY_METHODS,
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
        "__builtin_typed_array" => Some(typed_array_constructor(host, args)),
        "__builtin_typed_array_from" => Some(typed_array_from(host, args)),
        "__builtin_typed_array_of" => Some(create_typed_array_instance(host, args.to_vec())),
        "__builtin_typed_array_at" => Some(typed_array_at(host, this_value, args)),
        "__builtin_typed_array_set" => Some(typed_array_set(host, this_value, args)),
        "__builtin_typed_array_to_array" => Some(typed_array_to_array(host, this_value)),
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    (name == "__builtin_typed_array").then(|| typed_array_constructor(host, args))
}

fn typed_array_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    if let Some(length) = args
        .first()
        .and_then(|&value| to_f64(host.number_value(value)))
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
    {
        return create_typed_array_instance(
            host,
            std::iter::repeat_n(make_number(0.0), length as usize).collect(),
        );
    }

    typed_array_from(host, args)
}

fn typed_array_from<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(source) = args.first().copied() else {
        return create_typed_array_instance(host, Vec::new());
    };

    if let Some(values) = host.array_values(source) {
        return create_typed_array_instance(host, values);
    }

    if let Some(text) = host.string_text(source).map(str::to_owned) {
        let values = text
            .bytes()
            .map(|byte| make_number(byte as f64))
            .collect::<Vec<_>>();
        return create_typed_array_instance(host, values);
    }

    create_typed_array_instance(host, vec![source])
}

fn create_typed_array_instance<H: BuiltinHost>(host: &mut H, values: Vec<JSValue>) -> JSValue {
    let typed = host.create_object();
    let kind = host.intern_string("TypedArray");
    let data = create_array_from_values(host, values.clone());
    host.set_property(typed, KIND_PROP, kind);
    host.set_property(typed, DATA_PROP, data);
    host.set_property(typed, "length", make_number(values.len() as f64));
    install_methods(host, typed, INSTANCE_METHODS);
    typed
}

fn typed_array_at<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if host.string_text(host.get_property(this_value, KIND_PROP)) != Some("TypedArray") {
        return make_undefined();
    }

    let data = host.get_property(this_value, DATA_PROP);
    let values = host.array_values(data).unwrap_or_default();
    let index = args
        .first()
        .and_then(|&value| to_f64(host.number_value(value)))
        .map(|value| value.trunc() as isize)
        .unwrap_or(0);
    let index = if index < 0 {
        values.len().saturating_sub(index.unsigned_abs())
    } else {
        index as usize
    };
    values.get(index).copied().unwrap_or_else(make_undefined)
}

fn typed_array_set<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if host.string_text(host.get_property(this_value, KIND_PROP)) != Some("TypedArray") {
        return make_undefined();
    }

    let Some(source) = args
        .first()
        .copied()
        .and_then(|value| host.array_values(value))
    else {
        return make_undefined();
    };
    let offset = args
        .get(1)
        .and_then(|&value| to_f64(host.number_value(value)))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.trunc() as usize)
        .unwrap_or(0);

    let data = host.get_property(this_value, DATA_PROP);
    for (index, value) in source.into_iter().enumerate() {
        let _ = host.set_index(data, offset + index, value);
    }
    if let Some(values) = host.array_values(data) {
        host.set_property(this_value, "length", make_number(values.len() as f64));
    }
    make_undefined()
}

fn typed_array_to_array<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    if host.string_text(host.get_property(this_value, KIND_PROP)) != Some("TypedArray") {
        return host.create_array();
    }

    host.get_property(this_value, DATA_PROP)
}
