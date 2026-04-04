use std::collections::HashMap;
use std::time::Instant;

use value::{JSValue, make_bool, make_undefined};

mod array;
mod array_buffer;
mod atomics;
mod boolean;
mod collections;
mod data_view;
mod diagnostics;
mod eval;
mod function;
mod generator;
mod generator_function;
mod intl;
mod iterable;
mod math;
mod number;
mod object;
mod promise;
mod proxy;
mod reflect;
mod regexp;
mod serialization;
mod string;
mod symbol;
mod temporal;
mod time;
mod typed_array;
mod uri;

#[derive(Clone, Copy)]
pub struct BuiltinMethod {
    pub property_name: &'static str,
    pub native_name: &'static str,
}

impl BuiltinMethod {
    pub const fn new(property_name: &'static str, native_name: &'static str) -> Self {
        Self {
            property_name,
            native_name,
        }
    }
}

pub trait BuiltinHost {
    fn prepare_js_builtin_properties(&mut self, properties: &[String]);
    fn prepare_js_builtin_private_properties(&mut self, private_properties: &[String]);
    fn set_global(&mut self, global_slot: u16, value: JSValue);
    fn builtin_function(&mut self, native_name: &'static str) -> JSValue;

    fn create_object(&mut self) -> JSValue;
    fn create_array(&mut self) -> JSValue;
    fn get_property(&self, object: JSValue, name: &str) -> JSValue;
    fn get_own_property(&self, object: JSValue, name: &str) -> JSValue;
    fn set_property(&mut self, object: JSValue, name: &str, value: JSValue) -> JSValue;
    fn delete_property(&mut self, object: JSValue, name: &str) -> bool;
    fn has_property(&self, object: JSValue, name: &str) -> bool;
    fn has_own_property(&self, object: JSValue, name: &str) -> bool;
    fn get_property_value(&self, object: JSValue, key: JSValue) -> JSValue;
    fn get_own_property_value(&self, object: JSValue, key: JSValue) -> JSValue;
    fn set_property_value(&mut self, object: JSValue, key: JSValue, value: JSValue) -> JSValue;
    fn delete_property_value(&mut self, object: JSValue, key: JSValue) -> bool;
    fn has_property_value(&self, object: JSValue, key: JSValue) -> bool;
    fn has_own_property_value(&self, object: JSValue, key: JSValue) -> bool;
    fn own_property_names(&self, object: JSValue) -> Vec<String>;
    fn own_property_keys(&mut self, object: JSValue) -> Vec<JSValue>;
    fn get_index(&self, object: JSValue, index: usize) -> JSValue;
    fn set_index(&mut self, object: JSValue, index: usize, value: JSValue) -> JSValue;
    fn array_push(&mut self, object: JSValue, value: JSValue) -> JSValue;
    fn array_values(&self, value: JSValue) -> Option<Vec<JSValue>>;
    fn same_value(&self, lhs: JSValue, rhs: JSValue) -> bool;
    fn is_array(&self, value: JSValue) -> bool;
    fn is_object(&self, value: JSValue) -> bool;
    fn is_callable(&self, value: JSValue) -> bool;
    fn call_value(&mut self, callee: JSValue, this_value: JSValue, args: &[JSValue]) -> JSValue;
    fn construct_value(&mut self, callee: JSValue, args: &[JSValue]) -> JSValue;

    fn json_stringify(&mut self, value: JSValue) -> Result<String, String>;
    fn json_parse(&mut self, text: &str) -> Result<JSValue, String>;
    fn yaml_stringify(&mut self, value: JSValue) -> Result<String, String>;
    fn yaml_parse(&mut self, text: &str) -> Result<JSValue, String>;
    fn msgpack_encode(&mut self, value: JSValue) -> Result<Vec<u8>, String>;
    fn msgpack_decode(&mut self, bytes: &[u8]) -> Result<JSValue, String>;
    fn bin_encode(&mut self, value: JSValue) -> Result<Vec<u8>, String>;
    fn bin_decode(&mut self, bytes: &[u8]) -> Result<JSValue, String>;

    fn intern_string(&mut self, text: &str) -> JSValue;
    fn string_text<'a>(&'a self, value: JSValue) -> Option<&'a str>;
    fn is_symbol(&self, value: JSValue) -> bool;
    fn bytes_from_value(&self, value: JSValue) -> Option<Vec<u8>>;
    fn bytes_to_value(&mut self, bytes: &[u8]) -> JSValue;
    fn display_string(&mut self, value: JSValue) -> String;
    fn number_value(&mut self, value: JSValue) -> JSValue;
    fn is_truthy_value(&self, value: JSValue) -> bool;
    fn create_symbol(&mut self, description: Option<&str>) -> JSValue;
    fn symbol_for(&mut self, key: &str) -> JSValue;
    fn symbol_key_for(&self, value: JSValue) -> Option<String>;
    fn eval_source(&mut self, source: &str) -> Result<JSValue, String>;

    fn console_render_args(&mut self, args: &[JSValue]) -> String;
    fn console_write_line(&mut self, text: String, is_error: bool) -> JSValue;
    fn console_label_from_args(&mut self, args: &[JSValue]) -> String;
    fn console_elapsed_message(&self, label: &str, start: Instant, suffix: Option<&str>) -> String;
    fn console_time_start(&mut self, label: String);
    fn console_time_end(&mut self, label: &str) -> Option<Instant>;
    fn console_time_get(&self, label: &str) -> Option<Instant>;
    fn console_group_start(&mut self);
    fn console_group_end(&mut self);
    fn console_clear(&mut self);
    fn console_count_increment(&mut self, label: &str) -> usize;
}

pub fn install_js_builtins<H: BuiltinHost>(host: &mut H, names: &[String], properties: &[String], private_properties: &[String]) {
    host.prepare_js_builtin_properties(properties);
    host.prepare_js_builtin_private_properties(private_properties);

    let global_slots: HashMap<&str, u16> = names
        .iter()
        .enumerate()
        .filter_map(|(slot, name)| u16::try_from(slot).ok().map(|slot| (name.as_str(), slot)))
        .collect();

    array::install(host, &global_slots);
    array_buffer::install(host, &global_slots);
    atomics::install(host, &global_slots);
    boolean::install(host, &global_slots);
    collections::install(host, &global_slots);
    data_view::install(host, &global_slots);
    diagnostics::install(host, &global_slots);
    eval::install(host, &global_slots);
    function::install(host, &global_slots);
    generator::install(host, &global_slots);
    generator_function::install(host, &global_slots);
    intl::install(host, &global_slots);
    iterable::install(host, &global_slots);
    math::install(host, &global_slots);
    number::install(host, &global_slots);
    object::install(host, &global_slots);
    promise::install(host, &global_slots);
    proxy::install(host, &global_slots);
    reflect::install(host, &global_slots);
    regexp::install(host, &global_slots);
    serialization::install(host, &global_slots);
    string::install(host, &global_slots);
    symbol::install(host, &global_slots);
    temporal::install(host, &global_slots);
    time::install(host, &global_slots);
    typed_array::install(host, &global_slots);
    uri::install(host, &global_slots);
    let _ = install_global_function(
        host,
        &global_slots,
        "__qjs_get_template_object",
        "__qjs_get_template_object",
        &[],
    );
}

pub fn dispatch_builtin<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    dispatch_internal_builtin(host, name, callee_value, args)
        .or_else(|| serialization::dispatch(host, name, args))
        .or_else(|| time::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| diagnostics::dispatch(host, name, args))
        .or_else(|| math::dispatch(host, name, args))
        .or_else(|| number::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| object::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| array::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| boolean::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| string::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| symbol::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| eval::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| function::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| iterable::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| generator::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| generator_function::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| collections::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| promise::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| proxy::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| reflect::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| array_buffer::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| atomics::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| data_view::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| typed_array::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| regexp::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| intl::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| temporal::dispatch(host, name, callee_value, this_value, args))
        .or_else(|| uri::dispatch(host, name, callee_value, this_value, args))
}

const TEMPLATE_CACHE_PROP: &str = "__qjs_template_cache";
const TEMPLATE_FROZEN_PROP: &str = "__qjs_frozen";

fn dispatch_internal_builtin<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    callee_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__qjs_get_template_object" => Some(get_template_object(host, callee_value, args)),
        _ => None,
    }
}

fn get_template_object<H: BuiltinHost>(
    host: &mut H,
    callee_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    let key = args.first().copied().unwrap_or_else(make_undefined);
    let cache = {
        let existing = host.get_property(callee_value, TEMPLATE_CACHE_PROP);
        if host.is_object(existing) {
            existing
        } else {
            let created = host.create_object();
            host.set_property(callee_value, TEMPLATE_CACHE_PROP, created);
            created
        }
    };

    let cached = host.get_own_property_value(cache, key);
    if !cached.is_undefined() {
        return cached;
    }

    let cooked = args.get(1).copied().unwrap_or_else(make_undefined);
    let raw = args.get(2).copied().unwrap_or_else(make_undefined);

    if host.is_object(cooked) {
        host.set_property(cooked, "raw", raw);
        host.set_property(cooked, TEMPLATE_FROZEN_PROP, make_bool(true));
    }
    if host.is_object(raw) {
        host.set_property(raw, "raw", raw);
        host.set_property(raw, TEMPLATE_FROZEN_PROP, make_bool(true));
    }

    host.set_property_value(cache, key, cooked);
    cooked
}

pub fn dispatch_constructor<H: BuiltinHost>(
    host: &mut H,
    callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    time::construct(host, callee_value, name, args)
        .or_else(|| number::construct(host, callee_value, name, args))
        .or_else(|| object::construct(host, callee_value, name, args))
        .or_else(|| array::construct(host, callee_value, name, args))
        .or_else(|| boolean::construct(host, callee_value, name, args))
        .or_else(|| string::construct(host, callee_value, name, args))
        .or_else(|| function::construct(host, callee_value, name, args))
        .or_else(|| iterable::construct(host, callee_value, name, args))
        .or_else(|| generator::construct(host, callee_value, name, args))
        .or_else(|| generator_function::construct(host, callee_value, name, args))
        .or_else(|| collections::construct(host, callee_value, name, args))
        .or_else(|| promise::construct(host, callee_value, name, args))
        .or_else(|| proxy::construct(host, callee_value, name, args))
        .or_else(|| array_buffer::construct(host, callee_value, name, args))
        .or_else(|| data_view::construct(host, callee_value, name, args))
        .or_else(|| typed_array::construct(host, callee_value, name, args))
        .or_else(|| regexp::construct(host, callee_value, name, args))
        .or_else(|| intl::construct(host, callee_value, name, args))
        .or_else(|| temporal::construct(host, callee_value, name, args))
}

pub fn attach_callable_methods<H: BuiltinHost>(host: &mut H, target: JSValue) {
    function::attach_callable_methods(host, target);
}

pub fn attach_array_methods<H: BuiltinHost>(host: &mut H, target: JSValue) {
    array::attach_array_methods(host, target);
}

pub fn create_string_prototype<H: BuiltinHost>(host: &mut H) -> JSValue {
    string::create_string_prototype(host)
}

pub fn object_internal_prototype_name() -> &'static str {
    object::internal_prototype_name()
}

pub(crate) fn create_builtin_callable<H: BuiltinHost>(
    host: &mut H,
    native_name: &'static str,
) -> JSValue {
    let function = host.builtin_function(native_name);
    attach_callable_methods(host, function);
    function
}

pub(crate) fn install_global_object<H: BuiltinHost>(
    host: &mut H,
    global_slots: &HashMap<&str, u16>,
    global_name: &str,
    methods: &[BuiltinMethod],
) -> Option<JSValue> {
    let &slot = global_slots.get(global_name)?;
    let object = host.create_object();
    install_methods(host, object, methods);
    host.set_global(slot, object);
    Some(object)
}

pub(crate) fn install_global_function<H: BuiltinHost>(
    host: &mut H,
    global_slots: &HashMap<&str, u16>,
    global_name: &str,
    native_name: &'static str,
    methods: &[BuiltinMethod],
) -> Option<JSValue> {
    let &slot = global_slots.get(global_name)?;
    let function = create_builtin_callable(host, native_name);
    install_methods(host, function, methods);
    host.set_global(slot, function);
    Some(function)
}

pub(crate) fn install_methods<H: BuiltinHost>(
    host: &mut H,
    target: JSValue,
    methods: &[BuiltinMethod],
) {
    for method in methods {
        let function = host.builtin_function(method.native_name);
        host.set_property(target, method.property_name, function);
    }
}

pub(crate) fn create_array_from_values<H: BuiltinHost>(
    host: &mut H,
    values: impl IntoIterator<Item = JSValue>,
) -> JSValue {
    let array = host.create_array();
    for value in values {
        let _ = host.array_push(array, value);
    }
    array
}

pub(crate) fn filter_public_properties(names: Vec<String>) -> Vec<String> {
    names
        .into_iter()
        .filter(|name| !name.starts_with("__qjs_"))
        .collect()
}
