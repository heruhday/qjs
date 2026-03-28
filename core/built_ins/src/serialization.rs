use value::{JSValue, make_undefined};

use crate::{BuiltinHost, BuiltinMethod, BuiltinObject};

const JSON_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("stringify", "__builtin_json_stringify"),
    BuiltinMethod::new("parse", "__builtin_json_parse"),
];

const MSGPACK_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("encode", "__builtin_msgpack_encode"),
    BuiltinMethod::new("decode", "__builtin_msgpack_decode"),
];

const BIN_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("encode", "__builtin_bin_encode"),
    BuiltinMethod::new("decode", "__builtin_bin_decode"),
];

const YAML_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("stringify", "__builtin_yaml_stringify"),
    BuiltinMethod::new("parse", "__builtin_yaml_parse"),
];

pub(crate) const OBJECTS: &[BuiltinObject] = &[
    BuiltinObject::new("JSON", JSON_METHODS),
    BuiltinObject::new("Msgpack", MSGPACK_METHODS),
    BuiltinObject::new("Bin", BIN_METHODS),
    BuiltinObject::new("YAML", YAML_METHODS),
];

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_json_stringify" => Some(json_stringify(host, args)),
        "__builtin_json_parse" => Some(json_parse(host, args)),
        "__builtin_yaml_stringify" => Some(yaml_stringify(host, args)),
        "__builtin_yaml_parse" => Some(yaml_parse(host, args)),
        "__builtin_msgpack_encode" => Some(msgpack_encode(host, args)),
        "__builtin_msgpack_decode" => Some(msgpack_decode(host, args)),
        "__builtin_bin_encode" => Some(bin_encode(host, args)),
        "__builtin_bin_decode" => Some(bin_decode(host, args)),
        _ => None,
    }
}

fn json_stringify<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(value) = args.first().copied() else {
        return make_undefined();
    };

    match host.json_stringify(value) {
        Ok(text) => host.intern_string(&text),
        Err(_) => make_undefined(),
    }
}

fn json_parse<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(text) = args
        .first()
        .and_then(|value| host.string_text(*value).map(str::to_owned))
    else {
        return make_undefined();
    };

    host.json_parse(&text).unwrap_or_else(|_| make_undefined())
}

fn yaml_stringify<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(value) = args.first().copied() else {
        return make_undefined();
    };

    match host.yaml_stringify(value) {
        Ok(text) => host.intern_string(&text),
        Err(_) => make_undefined(),
    }
}

fn yaml_parse<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(text) = args
        .first()
        .and_then(|value| host.string_text(*value).map(str::to_owned))
    else {
        return make_undefined();
    };

    host.yaml_parse(&text).unwrap_or_else(|_| make_undefined())
}

fn msgpack_encode<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(value) = args.first().copied() else {
        return make_undefined();
    };

    match host.msgpack_encode(value) {
        Ok(bytes) => host.bytes_to_value(&bytes),
        Err(_) => make_undefined(),
    }
}

fn msgpack_decode<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(bytes) = args.first().and_then(|value| host.bytes_from_value(*value)) else {
        return make_undefined();
    };

    host.msgpack_decode(&bytes)
        .unwrap_or_else(|_| make_undefined())
}

fn bin_encode<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(value) = args.first().copied() else {
        return make_undefined();
    };

    match host.bin_encode(value) {
        Ok(bytes) => host.bytes_to_value(&bytes),
        Err(_) => make_undefined(),
    }
}

fn bin_decode<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(bytes) = args.first().and_then(|value| host.bytes_from_value(*value)) else {
        return make_undefined();
    };

    host.bin_decode(&bytes).unwrap_or_else(|_| make_undefined())
}
