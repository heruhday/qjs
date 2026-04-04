use std::collections::HashMap;

use value::{JSValue, make_undefined};

use crate::{BuiltinHost, BuiltinMethod, install_global_function};

const SYMBOL_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("for", "__builtin_symbol_for"),
    BuiltinMethod::new("keyFor", "__builtin_symbol_key_for"),
];

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(
        host,
        global_slots,
        "Symbol",
        "__builtin_symbol",
        SYMBOL_METHODS,
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
        "__builtin_symbol" => Some(symbol_constructor(host, args)),
        "__builtin_symbol_for" => Some(symbol_for(host, args)),
        "__builtin_symbol_key_for" => Some(symbol_key_for(host, args)),
        _ => None,
    }
}

fn symbol_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let description = args
        .first()
        .copied()
        .map(|value| host.display_string(value));
    host.create_symbol(description.as_deref())
}

fn symbol_for<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let key = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_default();
    host.symbol_for(&key)
}

fn symbol_key_for<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(value) = args.first().copied() else {
        return make_undefined();
    };
    match host.symbol_key_for(value) {
        Some(key) => host.intern_string(&key),
        None => make_undefined(),
    }
}
