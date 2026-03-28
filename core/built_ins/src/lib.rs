use std::collections::HashMap;
use std::time::Instant;

use value::JSValue;

mod diagnostics;
mod number;
mod serialization;
mod time;

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

#[derive(Clone, Copy)]
struct BuiltinObject {
    global_name: &'static str,
    methods: &'static [BuiltinMethod],
}

impl BuiltinObject {
    const fn new(global_name: &'static str, methods: &'static [BuiltinMethod]) -> Self {
        Self {
            global_name,
            methods,
        }
    }
}

pub trait BuiltinHost {
    fn prepare_js_builtin_properties(&mut self, properties: &[String]);
    fn install_builtin_object(&mut self, global_slot: u16, methods: &[BuiltinMethod]);

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
    fn bytes_from_value(&self, value: JSValue) -> Option<Vec<u8>>;
    fn bytes_to_value(&mut self, bytes: &[u8]) -> JSValue;
    fn display_string(&mut self, value: JSValue) -> String;
    fn number_value(&mut self, value: JSValue) -> JSValue;
    fn is_truthy_value(&self, value: JSValue) -> bool;

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

pub fn install_js_builtins<H: BuiltinHost>(host: &mut H, names: &[String], properties: &[String]) {
    host.prepare_js_builtin_properties(properties);

    let global_slots: HashMap<&str, u16> = names
        .iter()
        .enumerate()
        .filter_map(|(slot, name)| u16::try_from(slot).ok().map(|slot| (name.as_str(), slot)))
        .collect();

    install_group(host, &global_slots, serialization::OBJECTS);
    install_group(host, &global_slots, time::OBJECTS);
    install_group(host, &global_slots, diagnostics::OBJECTS);
}

pub fn dispatch_builtin<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    serialization::dispatch(host, name, args)
        .or_else(|| time::dispatch(host, name, args))
        .or_else(|| diagnostics::dispatch(host, name, args))
        .or_else(|| number::dispatch(host, name, this_value, args))
}

fn install_group<H: BuiltinHost>(
    host: &mut H,
    global_slots: &HashMap<&str, u16>,
    objects: &[BuiltinObject],
) {
    for builtin in objects {
        if let Some(&slot) = global_slots.get(builtin.global_name) {
            host.install_builtin_object(slot, builtin.methods);
        }
    }
}
