use std::collections::HashMap;

use value::{JSValue, make_bool, make_undefined};

use crate::{
    BuiltinHost, BuiltinMethod, attach_callable_methods, create_array_from_values,
    filter_public_properties, install_global_function,
};

const PROXY_METHODS: &[BuiltinMethod] =
    &[BuiltinMethod::new("revocable", "__builtin_proxy_revocable")];

const TARGET_PROP: &str = "__qjs_proxy_target";
const HANDLER_PROP: &str = "__qjs_proxy_handler";
const REVOKED_PROP: &str = "__qjs_proxy_revoked";
const KIND_PROP: &str = "__qjs_builtin_kind";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(
        host,
        global_slots,
        "Proxy",
        "__builtin_proxy",
        PROXY_METHODS,
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
        "__builtin_proxy" => Some(proxy_constructor(host, args)),
        "__builtin_proxy_revocable" => Some(proxy_revocable(host, args)),
        "__builtin_proxy_revoke" => Some(proxy_revoke(host, callee_value)),
        "__builtin_proxy_callable" => Some(proxy_call(host, callee_value, this_value, args)),
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
        "__builtin_proxy" => Some(proxy_constructor(host, args)),
        "__builtin_proxy_callable" => Some(proxy_construct(host, callee_value, args)),
        _ => None,
    }
}

pub(crate) fn is_proxy<H: BuiltinHost>(host: &H, value: JSValue) -> bool {
    host.string_text(host.get_property(value, KIND_PROP)) == Some("Proxy")
}

pub(crate) fn proxy_get<H: BuiltinHost>(host: &mut H, proxy: JSValue, key: JSValue) -> JSValue {
    if !is_proxy(host, proxy) || proxy_revoked_state(host, proxy) {
        return make_undefined();
    }

    let handler = host.get_property(proxy, HANDLER_PROP);
    let target = host.get_property(proxy, TARGET_PROP);
    let trap = host.get_property(handler, "get");
    if host.is_callable(trap) {
        return host.call_value(trap, handler, &[target, key, proxy]);
    }

    host.get_property_value(target, key)
}

pub(crate) fn proxy_set<H: BuiltinHost>(
    host: &mut H,
    proxy: JSValue,
    key: JSValue,
    value: JSValue,
) -> bool {
    if !is_proxy(host, proxy) || proxy_revoked_state(host, proxy) {
        return false;
    }

    let handler = host.get_property(proxy, HANDLER_PROP);
    let target = host.get_property(proxy, TARGET_PROP);
    let trap = host.get_property(handler, "set");
    if host.is_callable(trap) {
        let result = host.call_value(trap, handler, &[target, key, value, proxy]);
        return host.is_truthy_value(result);
    }

    host.set_property_value(target, key, value);
    true
}

pub(crate) fn proxy_has<H: BuiltinHost>(host: &mut H, proxy: JSValue, key: JSValue) -> bool {
    if !is_proxy(host, proxy) || proxy_revoked_state(host, proxy) {
        return false;
    }

    let handler = host.get_property(proxy, HANDLER_PROP);
    let target = host.get_property(proxy, TARGET_PROP);
    let trap = host.get_property(handler, "has");
    if host.is_callable(trap) {
        let result = host.call_value(trap, handler, &[target, key]);
        return host.is_truthy_value(result);
    }

    host.has_property_value(target, key)
}

pub(crate) fn proxy_delete_property<H: BuiltinHost>(
    host: &mut H,
    proxy: JSValue,
    key: JSValue,
) -> bool {
    if !is_proxy(host, proxy) || proxy_revoked_state(host, proxy) {
        return false;
    }

    let handler = host.get_property(proxy, HANDLER_PROP);
    let target = host.get_property(proxy, TARGET_PROP);
    let trap = host.get_property(handler, "deleteProperty");
    if host.is_callable(trap) {
        let result = host.call_value(trap, handler, &[target, key]);
        return host.is_truthy_value(result);
    }

    host.delete_property_value(target, key)
}

pub(crate) fn proxy_own_keys<H: BuiltinHost>(host: &mut H, proxy: JSValue) -> Vec<String> {
    if !is_proxy(host, proxy) || proxy_revoked_state(host, proxy) {
        return Vec::new();
    }

    let handler = host.get_property(proxy, HANDLER_PROP);
    let target = host.get_property(proxy, TARGET_PROP);
    let trap = host.get_property(handler, "ownKeys");
    if host.is_callable(trap) {
        let result = host.call_value(trap, handler, &[target]);
        if let Some(values) = host.array_values(result) {
            return values
                .into_iter()
                .map(|value| host.display_string(value))
                .collect();
        }
    }

    filter_public_properties(host.own_property_names(target))
}

fn proxy_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let target = args.first().copied().unwrap_or_else(make_undefined);
    let handler = args.get(1).copied().unwrap_or_else(make_undefined);

    let proxy = if host.is_callable(target) {
        let proxy = host.builtin_function("__builtin_proxy_callable");
        attach_callable_methods(host, proxy);
        proxy
    } else {
        host.create_object()
    };

    let kind = host.intern_string("Proxy");
    host.set_property(proxy, KIND_PROP, kind);
    host.set_property(proxy, TARGET_PROP, target);
    host.set_property(proxy, HANDLER_PROP, handler);
    host.set_property(proxy, REVOKED_PROP, make_bool(false));

    if host.is_object(target) {
        for name in filter_public_properties(host.own_property_names(target)) {
            let value = host.get_property(target, &name);
            host.set_property(proxy, &name, value);
        }
    }

    proxy
}

fn proxy_revocable<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let proxy = proxy_constructor(host, args);
    let revoke = host.builtin_function("__builtin_proxy_revoke");
    host.set_property(revoke, TARGET_PROP, proxy);

    let result = host.create_object();
    host.set_property(result, "proxy", proxy);
    host.set_property(result, "revoke", revoke);
    result
}

fn proxy_revoke<H: BuiltinHost>(host: &mut H, revoke: JSValue) -> JSValue {
    let proxy = host.get_property(revoke, TARGET_PROP);
    if is_proxy(host, proxy) {
        host.set_property(proxy, REVOKED_PROP, make_bool(true));
    }
    make_undefined()
}

fn proxy_call<H: BuiltinHost>(
    host: &mut H,
    proxy: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    if proxy_revoked_state(host, proxy) {
        return make_undefined();
    }

    let handler = host.get_property(proxy, HANDLER_PROP);
    let target = host.get_property(proxy, TARGET_PROP);
    let trap = host.get_property(handler, "apply");
    if host.is_callable(trap) {
        let args_array = create_array_from_values(host, args.iter().copied());
        return host.call_value(trap, handler, &[target, this_value, args_array]);
    }

    host.call_value(target, this_value, args)
}

fn proxy_construct<H: BuiltinHost>(host: &mut H, proxy: JSValue, args: &[JSValue]) -> JSValue {
    if proxy_revoked_state(host, proxy) {
        return host.create_object();
    }

    let handler = host.get_property(proxy, HANDLER_PROP);
    let target = host.get_property(proxy, TARGET_PROP);
    let trap = host.get_property(handler, "construct");
    if host.is_callable(trap) {
        let args_array = create_array_from_values(host, args.iter().copied());
        return host.call_value(trap, handler, &[target, args_array]);
    }

    host.construct_value(target, args)
}

fn proxy_revoked_state<H: BuiltinHost>(host: &H, proxy: JSValue) -> bool {
    host.is_truthy_value(host.get_property(proxy, REVOKED_PROP))
}
