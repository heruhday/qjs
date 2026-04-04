use std::collections::HashMap;

use value::JSValue;

use crate::{BuiltinHost, attach_callable_methods, install_global_function};

use super::{function, iterable};

const TARGET_PROP: &str = "__qjs_generator_function_target";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(
        host,
        global_slots,
        "GeneratorFunction",
        "__builtin_generator_function",
        &[],
    );
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_generator_function" => Some(generator_function_constructor(host, args)),
        "__builtin_generator_function_wrapper" => {
            Some(generator_wrapper_call(host, callee_value, this_value, args))
        }
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_generator_function" => Some(generator_function_constructor(host, args)),
        "__builtin_generator_function_wrapper" => {
            Some(generator_wrapper_construct(host, callee_value, args))
        }
        _ => None,
    }
}

fn generator_function_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let target = function::function_constructor(host, args);
    let wrapper = host.builtin_function("__builtin_generator_function_wrapper");
    attach_callable_methods(host, wrapper);
    host.set_property(wrapper, TARGET_PROP, target);
    wrapper
}

fn generator_wrapper_call<H: BuiltinHost>(
    host: &mut H,
    wrapper: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    let target = host.get_property(wrapper, TARGET_PROP);
    let result = host.call_value(target, this_value, args);
    iterable::iterator_from_value(host, Some(result), "Generator")
}

fn generator_wrapper_construct<H: BuiltinHost>(
    host: &mut H,
    wrapper: JSValue,
    args: &[JSValue],
) -> JSValue {
    let target = host.get_property(wrapper, TARGET_PROP);
    let result = host.construct_value(target, args);
    iterable::iterator_from_value(host, Some(result), "Generator")
}
