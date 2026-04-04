use std::collections::HashMap;

use value::{JSValue, make_bool, make_null, make_undefined};

use crate::{
    BuiltinHost, BuiltinMethod, create_array_from_values, create_builtin_callable,
    install_global_function,
};

const OBJECT_STATIC_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("create", "__builtin_object_create"),
    BuiltinMethod::new("setPrototypeOf", "__builtin_object_set_prototype_of"),
    BuiltinMethod::new("getPrototypeOf", "__builtin_object_get_prototype_of"),
    BuiltinMethod::new("defineProperty", "__builtin_object_define_property"),
    BuiltinMethod::new("defineProperties", "__builtin_object_define_properties"),
    BuiltinMethod::new("assign", "__builtin_object_assign"),
    BuiltinMethod::new("freeze", "__builtin_object_freeze"),
    BuiltinMethod::new("is", "__builtin_object_is"),
    BuiltinMethod::new("isFrozen", "__builtin_object_is_frozen"),
    BuiltinMethod::new("keys", "__builtin_object_keys"),
    BuiltinMethod::new("values", "__builtin_object_values"),
    BuiltinMethod::new("entries", "__builtin_object_entries"),
    BuiltinMethod::new(
        "getOwnPropertyDescriptor",
        "__builtin_object_get_own_property_descriptor",
    ),
    BuiltinMethod::new(
        "getOwnPropertyDescriptors",
        "__builtin_object_get_own_property_descriptors",
    ),
    BuiltinMethod::new(
        "getOwnPropertyNames",
        "__builtin_object_get_own_property_names",
    ),
    BuiltinMethod::new(
        "getOwnPropertySymbols",
        "__builtin_object_get_own_property_symbols",
    ),
    BuiltinMethod::new("hasOwn", "__builtin_object_has_own"),
    BuiltinMethod::new("fromEntries", "__builtin_object_from_entries"),
];

const OBJECT_INSTANCE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("hasOwnProperty", "__builtin_object_has_own_property"),
    BuiltinMethod::new(
        "propertyIsEnumerable",
        "__builtin_object_property_is_enumerable",
    ),
    BuiltinMethod::new("toString", "__builtin_object_to_string"),
    BuiltinMethod::new("toLocaleString", "__builtin_object_to_locale_string"),
    BuiltinMethod::new("valueOf", "__builtin_object_value_of"),
    BuiltinMethod::new("isPrototypeOf", "__builtin_object_is_prototype_of"),
];

const INTERNAL_PROTOTYPE_PROP: &str = "__qjs_object_prototype";
const BUILTIN_KIND_PROP: &str = "__qjs_builtin_kind";
const FROZEN_PROP: &str = "__qjs_frozen";

pub(crate) fn internal_prototype_name() -> &'static str {
    INTERNAL_PROTOTYPE_PROP
}

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let Some(constructor) = install_global_function(
        host,
        global_slots,
        "Object",
        "__builtin_object",
        OBJECT_STATIC_METHODS,
    ) else {
        return;
    };

    let prototype = create_object_prototype(host);
    host.set_property(constructor, "prototype", prototype);
}

pub(crate) fn attach_object_methods<H: BuiltinHost>(host: &mut H, target: JSValue) {
    for method in OBJECT_INSTANCE_METHODS {
        let function = create_builtin_callable(host, method.native_name);
        host.set_property(target, method.property_name, function);
    }
}

fn create_object_prototype<H: BuiltinHost>(host: &mut H) -> JSValue {
    let prototype = host.create_object();
    attach_object_methods(host, prototype);
    prototype
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    Some(match name {
        "__builtin_object" => object_constructor(host, args.first().copied()),
        "__builtin_object_create" => object_create(host, args),
        "__builtin_object_set_prototype_of" => object_set_prototype_of(host, args),
        "__builtin_object_get_prototype_of" => object_get_prototype_of(host, args),
        "__builtin_object_define_property" => object_define_property(host, args),
        "__builtin_object_define_properties" => object_define_properties(host, args),
        "__builtin_object_assign" => object_assign(host, args),
        "__builtin_object_freeze" => object_freeze(host, args),
        "__builtin_object_is" => object_is(host, args),
        "__builtin_object_is_frozen" => object_is_frozen(host, args),
        "__builtin_object_keys" => object_keys(host, args),
        "__builtin_object_values" => object_values(host, args),
        "__builtin_object_entries" => object_entries(host, args),
        "__builtin_object_get_own_property_descriptor" => {
            object_get_own_property_descriptor(host, args)
        }
        "__builtin_object_get_own_property_descriptors" => {
            object_get_own_property_descriptors(host, args)
        }
        "__builtin_object_get_own_property_names" => object_get_own_property_names(host, args),
        "__builtin_object_get_own_property_symbols" => object_get_own_property_symbols(host, args),
        "__builtin_object_has_own" => object_has_own(host, args),
        "__builtin_object_from_entries" => object_from_entries(host, args),
        "__builtin_object_has_own_property" => object_has_own_property(host, this_value, args),
        "__builtin_object_property_is_enumerable" => {
            object_property_is_enumerable(host, this_value, args)
        }
        "__builtin_object_to_string" => object_to_string(host, this_value),
        "__builtin_object_to_locale_string" => object_to_string(host, this_value),
        "__builtin_object_value_of" => object_value_of(host, this_value),
        "__builtin_object_is_prototype_of" => object_is_prototype_of(host, this_value, args),
        _ => return None,
    })
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    (name == "__builtin_object").then(|| object_constructor(host, args.first().copied()))
}

fn object_constructor<H: BuiltinHost>(host: &mut H, value: Option<JSValue>) -> JSValue {
    match value {
        Some(value) if host.is_object(value) => value,
        _ => host.create_object(),
    }
}

fn object_freeze<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let target = args.first().copied().unwrap_or_else(make_undefined);
    if host.is_object(target) {
        host.set_property(target, FROZEN_PROP, make_bool(true));
    }
    target
}

fn object_is_frozen<H: BuiltinHost>(host: &H, args: &[JSValue]) -> JSValue {
    let target = args.first().copied().unwrap_or_else(make_undefined);
    if !host.is_object(target) {
        return make_bool(true);
    }

    make_bool(
        host.get_own_property(target, FROZEN_PROP)
            .as_bool()
            .unwrap_or(false),
    )
}

fn own_public_string_keys<H: BuiltinHost>(host: &mut H, object: JSValue) -> Vec<JSValue> {
    let mut keys = host
        .own_property_keys(object)
        .into_iter()
        .filter(|key| {
            host.string_text(*key)
                .is_some_and(|name| !name.starts_with("__qjs_"))
        })
        .collect::<Vec<_>>();

    if host.is_array(object) {
        let length_key = host.intern_string("length");
        let has_length = keys
            .iter()
            .copied()
            .any(|key| host.string_text(key) == Some("length"));
        if !has_length && host.has_own_property(object, "length") {
            keys.push(length_key);
        }
    }

    keys
}

fn get_internal_prototype<H: BuiltinHost>(host: &H, object: JSValue) -> JSValue {
    host.get_own_property(object, INTERNAL_PROTOTYPE_PROP)
}

fn set_internal_prototype<H: BuiltinHost>(host: &mut H, object: JSValue, prototype: JSValue) {
    host.set_property(object, INTERNAL_PROTOTYPE_PROP, prototype);
}

fn prototype_chain_contains<H: BuiltinHost>(host: &H, start: JSValue, target: JSValue) -> bool {
    let mut current = start;
    while host.is_object(current) {
        if host.same_value(current, target) {
            return true;
        }

        let next = get_internal_prototype(host, current);
        if next.is_null() || next.is_undefined() || !host.is_object(next) {
            return false;
        }
        current = next;
    }

    false
}

fn descriptor_value<H: BuiltinHost>(host: &H, descriptor: JSValue) -> JSValue {
    if host.is_object(descriptor) && host.has_own_property(descriptor, "value") {
        return host.get_own_property(descriptor, "value");
    }

    make_undefined()
}

fn create_data_descriptor<H: BuiltinHost>(host: &mut H, value: JSValue) -> JSValue {
    let descriptor = host.create_object();
    host.set_property(descriptor, "value", value);
    host.set_property(descriptor, "writable", make_bool(true));
    host.set_property(descriptor, "enumerable", make_bool(true));
    host.set_property(descriptor, "configurable", make_bool(true));
    descriptor
}

fn object_create<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let proto = args.first().copied().unwrap_or_else(make_undefined);
    if !proto.is_null() && !host.is_object(proto) {
        return make_undefined();
    }

    let object = host.create_object();
    set_internal_prototype(host, object, proto);

    if let Some(&properties) = args.get(1) {
        let _ = apply_object_define_properties(host, object, properties);
    }

    object
}

fn object_get_prototype_of<H: BuiltinHost>(host: &H, args: &[JSValue]) -> JSValue {
    let Some(object) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return make_undefined();
    };

    let prototype = get_internal_prototype(host, object);
    if prototype.is_undefined() {
        make_null()
    } else {
        prototype
    }
}

fn object_set_prototype_of<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let target = args.first().copied().unwrap_or_else(make_undefined);
    if !host.is_object(target) {
        return target;
    }

    let proto = args.get(1).copied().unwrap_or_else(make_undefined);
    if !proto.is_null() && !host.is_object(proto) {
        return target;
    }

    if host.is_object(proto)
        && (host.same_value(target, proto) || prototype_chain_contains(host, proto, target))
    {
        return target;
    }

    set_internal_prototype(host, target, proto);
    target
}

fn object_define_property<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(object) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return make_undefined();
    };

    let key = args.get(1).copied().unwrap_or_else(make_undefined);
    let descriptor = args.get(2).copied().unwrap_or_else(make_undefined);
    let value = descriptor_value(host, descriptor);
    host.set_property_value(object, key, value);
    object
}

fn object_define_properties<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(object) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return make_undefined();
    };

    let properties = args.get(1).copied().unwrap_or_else(make_undefined);
    let _ = apply_object_define_properties(host, object, properties);
    object
}

fn apply_object_define_properties<H: BuiltinHost>(
    host: &mut H,
    object: JSValue,
    properties: JSValue,
) -> JSValue {
    if !host.is_object(properties) {
        return object;
    }

    for key in host.own_property_keys(properties) {
        if host
            .string_text(key)
            .is_some_and(|name| name.starts_with("__qjs_"))
        {
            continue;
        }

        let descriptor = host.get_property_value(properties, key);
        let value = descriptor_value(host, descriptor);
        host.set_property_value(object, key, value);
    }

    object
}

fn object_assign<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let target = object_constructor(host, args.first().copied());

    for source in args.iter().copied().skip(1) {
        if !host.is_object(source) {
            continue;
        }

        for key in own_public_string_keys(host, source) {
            let value = host.get_property_value(source, key);
            let _ = host.set_property_value(target, key, value);
        }
    }

    target
}

fn object_is<H: BuiltinHost>(host: &H, args: &[JSValue]) -> JSValue {
    let lhs = args.first().copied().unwrap_or_else(make_undefined);
    let rhs = args.get(1).copied().unwrap_or_else(make_undefined);
    make_bool(host.same_value(lhs, rhs))
}

fn object_keys<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(object) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return host.create_array();
    };

    let keys = own_public_string_keys(host, object);
    create_array_from_values(host, keys)
}

fn object_values<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(object) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return host.create_array();
    };

    let values = own_public_string_keys(host, object)
        .into_iter()
        .map(|key| host.get_property_value(object, key))
        .collect::<Vec<_>>();
    create_array_from_values(host, values)
}

fn object_entries<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(object) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return host.create_array();
    };

    let entries = own_public_string_keys(host, object)
        .into_iter()
        .map(|key| {
            let value = host.get_property_value(object, key);
            create_array_from_values(host, [key, value])
        })
        .collect::<Vec<_>>();
    create_array_from_values(host, entries)
}

fn object_get_own_property_descriptor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(object) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return make_undefined();
    };

    let key = args.get(1).copied().unwrap_or_else(make_undefined);
    if !host.has_own_property_value(object, key) {
        return make_undefined();
    }

    create_data_descriptor(host, host.get_own_property_value(object, key))
}

fn object_get_own_property_descriptors<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(object) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return host.create_object();
    };

    let out = host.create_object();
    for key in host.own_property_keys(object) {
        if host
            .string_text(key)
            .is_some_and(|name| name.starts_with("__qjs_"))
        {
            continue;
        }

        let descriptor = create_data_descriptor(host, host.get_own_property_value(object, key));
        host.set_property_value(out, key, descriptor);
    }
    out
}

fn object_get_own_property_names<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(object) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return host.create_array();
    };

    let keys = own_public_string_keys(host, object);
    create_array_from_values(host, keys)
}

fn object_get_own_property_symbols<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(object) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return host.create_array();
    };

    let keys = host
        .own_property_keys(object)
        .into_iter()
        .filter(|key| host.is_symbol(*key))
        .collect::<Vec<_>>();
    create_array_from_values(host, keys)
}

fn object_has_own<H: BuiltinHost>(host: &H, args: &[JSValue]) -> JSValue {
    let Some(object) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return make_bool(false);
    };
    let key = args.get(1).copied().unwrap_or_else(make_undefined);

    make_bool(host.has_own_property_value(object, key))
}

fn object_from_entries<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(source) = args.first().copied() else {
        return host.create_object();
    };

    let object = host.create_object();
    for entry in host.array_values(source).unwrap_or_default() {
        let values = host.array_values(entry).unwrap_or_default();
        let key = values.first().copied().unwrap_or_else(make_undefined);
        let value = values.get(1).copied().unwrap_or_else(make_undefined);
        host.set_property_value(object, key, value);
    }

    object
}

fn object_has_own_property<H: BuiltinHost>(
    host: &H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    if !host.is_object(this_value) {
        return make_bool(false);
    }

    let key = args.first().copied().unwrap_or_else(make_undefined);
    make_bool(host.has_own_property_value(this_value, key))
}

fn object_property_is_enumerable<H: BuiltinHost>(
    host: &H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    object_has_own_property(host, this_value, args)
}

fn object_to_string<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    let tag = if this_value.is_undefined() {
        "[object Undefined]".to_owned()
    } else if this_value.is_null() {
        "[object Null]".to_owned()
    } else if host.is_array(this_value) {
        "[object Array]".to_owned()
    } else if host.is_callable(this_value) {
        "[object Function]".to_owned()
    } else if host.is_symbol(this_value) {
        "[object Symbol]".to_owned()
    } else if let Some(kind) = host.string_text(host.get_property(this_value, BUILTIN_KIND_PROP)) {
        format!("[object {kind}]")
    } else {
        "[object Object]".to_owned()
    };

    host.intern_string(&tag)
}

fn object_value_of<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    if host.is_object(this_value) {
        this_value
    } else {
        object_constructor(host, Some(this_value))
    }
}

fn object_is_prototype_of<H: BuiltinHost>(
    host: &H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    let Some(mut current) = args.first().copied().filter(|value| host.is_object(*value)) else {
        return make_bool(false);
    };

    if !host.is_object(this_value) {
        return make_bool(false);
    }

    loop {
        let prototype = get_internal_prototype(host, current);
        if prototype.is_null() || prototype.is_undefined() || !host.is_object(prototype) {
            return make_bool(false);
        }
        if host.same_value(prototype, this_value) {
            return make_bool(true);
        }
        current = prototype;
    }
}
