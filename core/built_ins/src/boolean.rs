use std::collections::HashMap;

use value::{JSValue, make_bool, make_undefined};

use crate::{BuiltinHost, install_global_function};

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(host, global_slots, "Boolean", "__builtin_boolean", &[]);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    _this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    (name == "__builtin_boolean").then(|| {
        make_bool(
            args.first()
                .copied()
                .is_some_and(|value| host.is_truthy_value(value)),
        )
    })
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    dispatch(host, name, make_undefined(), make_undefined(), args)
}
