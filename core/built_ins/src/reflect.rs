use std::collections::HashMap;

use value::{JSValue, make_bool, make_undefined};

use crate::{BuiltinHost, BuiltinMethod, create_array_from_values, install_global_object};

use super::proxy;

const REFLECT_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("apply", "__builtin_reflect_apply"),
    BuiltinMethod::new("construct", "__builtin_reflect_construct"),
    BuiltinMethod::new("get", "__builtin_reflect_get"),
    BuiltinMethod::new("set", "__builtin_reflect_set"),
    BuiltinMethod::new("has", "__builtin_reflect_has"),
    BuiltinMethod::new("deleteProperty", "__builtin_reflect_delete_property"),
    BuiltinMethod::new("ownKeys", "__builtin_reflect_own_keys"),
];

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_object(host, global_slots, "Reflect", REFLECT_METHODS);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    _this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    Some(match name {
        "__builtin_reflect_apply" => reflect_apply(host, args),
        "__builtin_reflect_construct" => reflect_construct(host, args),
        "__builtin_reflect_get" => reflect_get(host, args),
        "__builtin_reflect_set" => reflect_set(host, args),
        "__builtin_reflect_has" => reflect_has(host, args),
        "__builtin_reflect_delete_property" => reflect_delete_property(host, args),
        "__builtin_reflect_own_keys" => reflect_own_keys(host, args),
        _ => return None,
    })
}

fn reflect_apply<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(target) = args.first().copied() else {
        return make_undefined();
    };
    let this_arg = args.get(1).copied().unwrap_or_else(make_undefined);
    let apply_args = args
        .get(2)
        .copied()
        .and_then(|value| host.array_values(value))
        .unwrap_or_default();
    host.call_value(target, this_arg, &apply_args)
}

fn reflect_construct<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(target) = args.first().copied() else {
        return host.create_object();
    };
    let construct_args = args
        .get(1)
        .copied()
        .and_then(|value| host.array_values(value))
        .unwrap_or_default();
    host.construct_value(target, &construct_args)
}

fn reflect_get<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(target) = args.first().copied() else {
        return make_undefined();
    };
    let key = args.get(1).copied().unwrap_or_else(make_undefined);
    if proxy::is_proxy(host, target) {
        return proxy::proxy_get(host, target, key);
    }
    host.get_property_value(target, key)
}

fn reflect_set<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(target) = args.first().copied() else {
        return make_bool(false);
    };
    let key = args.get(1).copied().unwrap_or_else(make_undefined);
    let value = args.get(2).copied().unwrap_or_else(make_undefined);
    let ok = if proxy::is_proxy(host, target) {
        proxy::proxy_set(host, target, key, value)
    } else {
        host.set_property_value(target, key, value);
        true
    };
    make_bool(ok)
}

fn reflect_has<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(target) = args.first().copied() else {
        return make_bool(false);
    };
    let key = args.get(1).copied().unwrap_or_else(make_undefined);
    let ok = if proxy::is_proxy(host, target) {
        proxy::proxy_has(host, target, key)
    } else {
        host.has_property_value(target, key)
    };
    make_bool(ok)
}

fn reflect_delete_property<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(target) = args.first().copied() else {
        return make_bool(false);
    };
    let key = args.get(1).copied().unwrap_or_else(make_undefined);
    let ok = if proxy::is_proxy(host, target) {
        proxy::proxy_delete_property(host, target, key)
    } else {
        host.delete_property_value(target, key)
    };
    make_bool(ok)
}

fn reflect_own_keys<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(target) = args.first().copied() else {
        return host.create_array();
    };
    if proxy::is_proxy(host, target) {
        let values = proxy::proxy_own_keys(host, target)
            .into_iter()
            .map(|key| host.intern_string(&key))
            .collect::<Vec<_>>();
        return create_array_from_values(host, values);
    }

    let keys = host.own_property_keys(target);
    create_array_from_values(host, keys)
}
