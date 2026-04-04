use std::collections::HashMap;

use value::{JSValue, make_bool, make_number, make_undefined};

use crate::{
    BuiltinHost, BuiltinMethod, create_array_from_values, install_global_function, install_methods,
};

const MAP_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("get", "__builtin_map_get"),
    BuiltinMethod::new("set", "__builtin_map_set"),
    BuiltinMethod::new("has", "__builtin_map_has"),
    BuiltinMethod::new("delete", "__builtin_map_delete"),
    BuiltinMethod::new("clear", "__builtin_map_clear"),
];

const SET_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("add", "__builtin_set_add"),
    BuiltinMethod::new("has", "__builtin_set_has"),
    BuiltinMethod::new("delete", "__builtin_set_delete"),
    BuiltinMethod::new("clear", "__builtin_set_clear"),
];

const KIND_PROP: &str = "__qjs_builtin_kind";
const ENTRIES_PROP: &str = "__qjs_collection_entries";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(host, global_slots, "Map", "__builtin_map", &[]);
    let _ = install_global_function(host, global_slots, "Set", "__builtin_set", &[]);
    let _ = install_global_function(host, global_slots, "WeakMap", "__builtin_weak_map", &[]);
    let _ = install_global_function(host, global_slots, "WeakSet", "__builtin_weak_set", &[]);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_map" => Some(map_constructor(host, args, "Map")),
        "__builtin_set" => Some(set_constructor(host, args, "Set")),
        "__builtin_weak_map" => Some(map_constructor(host, args, "WeakMap")),
        "__builtin_weak_set" => Some(set_constructor(host, args, "WeakSet")),
        "__builtin_map_get" => Some(map_get(host, this_value, args)),
        "__builtin_map_set" => Some(map_set(host, this_value, args)),
        "__builtin_map_has" => Some(map_has(host, this_value, args)),
        "__builtin_map_delete" => Some(map_delete(host, this_value, args)),
        "__builtin_map_clear" => Some(map_clear(host, this_value)),
        "__builtin_set_add" => Some(set_add(host, this_value, args)),
        "__builtin_set_has" => Some(set_has(host, this_value, args)),
        "__builtin_set_delete" => Some(set_delete(host, this_value, args)),
        "__builtin_set_clear" => Some(set_clear(host, this_value)),
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_map" => Some(map_constructor(host, args, "Map")),
        "__builtin_set" => Some(set_constructor(host, args, "Set")),
        "__builtin_weak_map" => Some(map_constructor(host, args, "WeakMap")),
        "__builtin_weak_set" => Some(set_constructor(host, args, "WeakSet")),
        _ => None,
    }
}

fn map_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue], kind: &str) -> JSValue {
    let map = host.create_object();
    let kind_value = host.intern_string(kind);
    let entries = host.create_array();
    host.set_property(map, KIND_PROP, kind_value);
    host.set_property(map, ENTRIES_PROP, entries);
    host.set_property(map, "size", make_number(0.0));
    install_methods(host, map, MAP_METHODS);

    if let Some(iterable) = args
        .first()
        .copied()
        .and_then(|value| host.array_values(value))
    {
        for pair in iterable {
            if let Some(values) = host.array_values(pair) {
                let key = values.first().copied().unwrap_or_else(make_undefined);
                let value = values.get(1).copied().unwrap_or_else(make_undefined);
                let _ = map_set_impl(host, map, key, value);
            }
        }
    }

    map
}

fn set_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue], kind: &str) -> JSValue {
    let set = host.create_object();
    let kind_value = host.intern_string(kind);
    let entries = host.create_array();
    host.set_property(set, KIND_PROP, kind_value);
    host.set_property(set, ENTRIES_PROP, entries);
    host.set_property(set, "size", make_number(0.0));
    install_methods(host, set, SET_METHODS);

    if let Some(iterable) = args
        .first()
        .copied()
        .and_then(|value| host.array_values(value))
    {
        for value in iterable {
            let _ = set_add_impl(host, set, value);
        }
    }

    set
}

fn collection_kind_matches<H: BuiltinHost>(
    host: &H,
    this_value: JSValue,
    expected: &[&str],
) -> bool {
    let Some(kind) = host.string_text(host.get_property(this_value, KIND_PROP)) else {
        return false;
    };

    expected.contains(&kind)
}

fn map_entries<H: BuiltinHost>(host: &H, this_value: JSValue) -> Vec<JSValue> {
    host.array_values(host.get_property(this_value, ENTRIES_PROP))
        .unwrap_or_default()
}

fn update_map_entries<H: BuiltinHost>(host: &mut H, this_value: JSValue, entries: Vec<JSValue>) {
    let entries_value = create_array_from_values(host, entries.clone());
    host.set_property(this_value, ENTRIES_PROP, entries_value);
    host.set_property(this_value, "size", make_number((entries.len() / 2) as f64));
}

fn set_entries<H: BuiltinHost>(host: &H, this_value: JSValue) -> Vec<JSValue> {
    host.array_values(host.get_property(this_value, ENTRIES_PROP))
        .unwrap_or_default()
}

fn update_set_entries<H: BuiltinHost>(host: &mut H, this_value: JSValue, entries: Vec<JSValue>) {
    let entries_value = create_array_from_values(host, entries.clone());
    host.set_property(this_value, ENTRIES_PROP, entries_value);
    host.set_property(this_value, "size", make_number(entries.len() as f64));
}

fn map_get<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !collection_kind_matches(host, this_value, &["Map", "WeakMap"]) {
        return make_undefined();
    }

    let key = args.first().copied().unwrap_or_else(make_undefined);
    for chunk in map_entries(host, this_value).chunks_exact(2) {
        if host.same_value(chunk[0], key) {
            return chunk[1];
        }
    }
    make_undefined()
}

fn map_set<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !collection_kind_matches(host, this_value, &["Map", "WeakMap"]) {
        return make_undefined();
    }

    let key = args.first().copied().unwrap_or_else(make_undefined);
    let value = args.get(1).copied().unwrap_or_else(make_undefined);
    map_set_impl(host, this_value, key, value)
}

fn map_set_impl<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    key: JSValue,
    value: JSValue,
) -> JSValue {
    let mut entries = map_entries(host, this_value);
    for chunk in entries.chunks_exact_mut(2) {
        if host.same_value(chunk[0], key) {
            chunk[1] = value;
            update_map_entries(host, this_value, entries);
            return this_value;
        }
    }

    entries.push(key);
    entries.push(value);
    update_map_entries(host, this_value, entries);
    this_value
}

fn map_has<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !collection_kind_matches(host, this_value, &["Map", "WeakMap"]) {
        return make_bool(false);
    }

    let key = args.first().copied().unwrap_or_else(make_undefined);
    make_bool(
        map_entries(host, this_value)
            .chunks_exact(2)
            .any(|chunk| host.same_value(chunk[0], key)),
    )
}

fn map_delete<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !collection_kind_matches(host, this_value, &["Map", "WeakMap"]) {
        return make_bool(false);
    }

    let key = args.first().copied().unwrap_or_else(make_undefined);
    let mut changed = false;
    let mut next = Vec::new();
    for chunk in map_entries(host, this_value).chunks_exact(2) {
        if host.same_value(chunk[0], key) {
            changed = true;
            continue;
        }
        next.push(chunk[0]);
        next.push(chunk[1]);
    }
    if changed {
        update_map_entries(host, this_value, next);
    }
    make_bool(changed)
}

fn map_clear<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    if collection_kind_matches(host, this_value, &["Map", "WeakMap"]) {
        update_map_entries(host, this_value, Vec::new());
    }
    make_undefined()
}

fn set_add<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !collection_kind_matches(host, this_value, &["Set", "WeakSet"]) {
        return make_undefined();
    }

    let value = args.first().copied().unwrap_or_else(make_undefined);
    set_add_impl(host, this_value, value)
}

fn set_add_impl<H: BuiltinHost>(host: &mut H, this_value: JSValue, value: JSValue) -> JSValue {
    let mut entries = set_entries(host, this_value);
    if !entries
        .iter()
        .copied()
        .any(|entry| host.same_value(entry, value))
    {
        entries.push(value);
        update_set_entries(host, this_value, entries);
    }
    this_value
}

fn set_has<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !collection_kind_matches(host, this_value, &["Set", "WeakSet"]) {
        return make_bool(false);
    }

    let value = args.first().copied().unwrap_or_else(make_undefined);
    make_bool(
        set_entries(host, this_value)
            .into_iter()
            .any(|entry| host.same_value(entry, value)),
    )
}

fn set_delete<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !collection_kind_matches(host, this_value, &["Set", "WeakSet"]) {
        return make_bool(false);
    }

    let value = args.first().copied().unwrap_or_else(make_undefined);
    let entries = set_entries(host, this_value);
    let next = entries
        .iter()
        .copied()
        .filter(|entry| !host.same_value(*entry, value))
        .collect::<Vec<_>>();
    let changed = next.len() != entries.len();
    if changed {
        update_set_entries(host, this_value, next);
    }
    make_bool(changed)
}

fn set_clear<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    if collection_kind_matches(host, this_value, &["Set", "WeakSet"]) {
        update_set_entries(host, this_value, Vec::new());
    }
    make_undefined()
}
