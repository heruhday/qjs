use std::collections::HashMap;

use value::JSValue;

use crate::{BuiltinHost, BuiltinMethod, install_global_function};

use super::iterable;

const GENERATOR_STATIC_METHODS: &[BuiltinMethod] =
    &[BuiltinMethod::new("from", "__builtin_generator_from")];

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(
        host,
        global_slots,
        "Generator",
        "__builtin_generator",
        GENERATOR_STATIC_METHODS,
    );
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    _this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_generator" | "__builtin_generator_from" => Some(iterable::iterator_from_value(
            host,
            args.first().copied(),
            "Generator",
        )),
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    matches!(name, "__builtin_generator" | "__builtin_generator_from")
        .then(|| iterable::iterator_from_value(host, args.first().copied(), "Generator"))
}
