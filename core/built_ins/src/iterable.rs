use std::collections::HashMap;

use value::{JSValue, make_bool, make_number, make_undefined, to_f64};

use crate::{
    BuiltinHost, BuiltinMethod, create_array_from_values, install_global_function, install_methods,
};

const ITERATOR_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("next", "__builtin_iterator_next"),
    BuiltinMethod::new("return", "__builtin_iterator_return"),
    BuiltinMethod::new("throw", "__builtin_iterator_throw"),
    BuiltinMethod::new("toArray", "__builtin_iterator_to_array"),
];

const ITERATOR_STATIC_METHODS: &[BuiltinMethod] =
    &[BuiltinMethod::new("from", "__builtin_iterator_from")];

const KIND_PROP: &str = "__qjs_builtin_kind";
const VALUES_PROP: &str = "__qjs_iterator_values";
const INDEX_PROP: &str = "__qjs_iterator_index";
const DONE_PROP: &str = "__qjs_iterator_done";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(
        host,
        global_slots,
        "Iterator",
        "__builtin_iterator",
        ITERATOR_STATIC_METHODS,
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
        "__builtin_iterator" | "__builtin_iterator_from" => {
            Some(iterator_from_value(host, args.first().copied(), "Iterator"))
        }
        "__builtin_iterator_next" => Some(iterator_next(host, this_value)),
        "__builtin_iterator_return" => Some(iterator_return(host, this_value, args)),
        "__builtin_iterator_throw" => Some(iterator_throw(host, this_value, args)),
        "__builtin_iterator_to_array" => Some(iterator_to_array(host, this_value)),
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    matches!(name, "__builtin_iterator" | "__builtin_iterator_from")
        .then(|| iterator_from_value(host, args.first().copied(), "Iterator"))
}

pub(crate) fn iterator_from_value<H: BuiltinHost>(
    host: &mut H,
    value: Option<JSValue>,
    kind: &str,
) -> JSValue {
    let iterator = host.create_object();
    let kind_value = host.intern_string(kind);
    let values = values_from_iterable(host, value);
    let values_array = create_array_from_values(host, values);
    host.set_property(iterator, KIND_PROP, kind_value);
    host.set_property(iterator, VALUES_PROP, values_array);
    host.set_property(iterator, INDEX_PROP, make_number(0.0));
    host.set_property(iterator, DONE_PROP, make_bool(false));
    install_methods(host, iterator, ITERATOR_METHODS);
    iterator
}

pub(crate) fn is_iterator_instance<H: BuiltinHost>(host: &H, value: JSValue) -> bool {
    matches!(
        host.string_text(host.get_property(value, KIND_PROP)),
        Some("Iterator" | "Generator")
    )
}

pub(crate) fn remaining_iterator_values<H: BuiltinHost>(
    host: &mut H,
    iterator: JSValue,
) -> Vec<JSValue> {
    let values = host
        .array_values(host.get_property(iterator, VALUES_PROP))
        .unwrap_or_default();
    let index = iterator_index(host, iterator).min(values.len());
    values[index..].to_vec()
}

fn values_from_iterable<H: BuiltinHost>(host: &mut H, value: Option<JSValue>) -> Vec<JSValue> {
    let Some(value) = value else {
        return Vec::new();
    };

    if is_iterator_instance(host, value) {
        return remaining_iterator_values(host, value);
    }

    if let Some(values) = host.array_values(value) {
        return values;
    }

    if let Some(text) = host.string_text(value).map(str::to_owned) {
        return text
            .chars()
            .map(|ch| host.intern_string(&ch.to_string()))
            .collect();
    }

    if value.is_undefined() || value.is_null() {
        return Vec::new();
    }

    vec![value]
}

fn iterator_next<H: BuiltinHost>(host: &mut H, iterator: JSValue) -> JSValue {
    if !is_iterator_instance(host, iterator) {
        return iter_result(host, make_undefined(), true);
    }

    if host.is_truthy_value(host.get_property(iterator, DONE_PROP)) {
        return iter_result(host, make_undefined(), true);
    }

    let values = host
        .array_values(host.get_property(iterator, VALUES_PROP))
        .unwrap_or_default();
    let index = iterator_index(host, iterator);
    if index >= values.len() {
        host.set_property(iterator, DONE_PROP, make_bool(true));
        return iter_result(host, make_undefined(), true);
    }

    host.set_property(iterator, INDEX_PROP, make_number((index + 1) as f64));
    iter_result(host, values[index], false)
}

fn iterator_return<H: BuiltinHost>(host: &mut H, iterator: JSValue, args: &[JSValue]) -> JSValue {
    if is_iterator_instance(host, iterator) {
        let len = host
            .array_values(host.get_property(iterator, VALUES_PROP))
            .map(|values| values.len())
            .unwrap_or(0);
        host.set_property(iterator, INDEX_PROP, make_number(len as f64));
        host.set_property(iterator, DONE_PROP, make_bool(true));
    }

    iter_result(
        host,
        args.first().copied().unwrap_or_else(make_undefined),
        true,
    )
}

fn iterator_throw<H: BuiltinHost>(host: &mut H, iterator: JSValue, args: &[JSValue]) -> JSValue {
    iterator_return(host, iterator, args)
}

fn iterator_to_array<H: BuiltinHost>(host: &mut H, iterator: JSValue) -> JSValue {
    let remaining = if is_iterator_instance(host, iterator) {
        remaining_iterator_values(host, iterator)
    } else {
        Vec::new()
    };
    if is_iterator_instance(host, iterator) {
        let len = host
            .array_values(host.get_property(iterator, VALUES_PROP))
            .map(|values| values.len())
            .unwrap_or(0);
        host.set_property(iterator, INDEX_PROP, make_number(len as f64));
        host.set_property(iterator, DONE_PROP, make_bool(true));
    }
    create_array_from_values(host, remaining)
}

fn iter_result<H: BuiltinHost>(host: &mut H, value: JSValue, done: bool) -> JSValue {
    let result = host.create_object();
    host.set_property(result, "value", value);
    host.set_property(result, "done", make_bool(done));
    result
}

fn iterator_index<H: BuiltinHost>(host: &mut H, iterator: JSValue) -> usize {
    to_f64(host.number_value(host.get_property(iterator, INDEX_PROP)))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.trunc() as usize)
        .unwrap_or(0)
}
