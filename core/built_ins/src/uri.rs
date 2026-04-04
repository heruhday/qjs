use std::collections::HashMap;

use value::JSValue;

use crate::{BuiltinHost, install_global_function};

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(host, global_slots, "encodeURI", "__builtin_encode_uri", &[]);
    let _ = install_global_function(
        host,
        global_slots,
        "encodeURIComponent",
        "__builtin_encode_uri_component",
        &[],
    );
    let _ = install_global_function(host, global_slots, "decodeURI", "__builtin_decode_uri", &[]);
    let _ = install_global_function(
        host,
        global_slots,
        "decodeURIComponent",
        "__builtin_decode_uri_component",
        &[],
    );
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    _this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    let input = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_default();

    let output = match name {
        "__builtin_encode_uri" => encode_uri_text(&input, false),
        "__builtin_encode_uri_component" => encode_uri_text(&input, true),
        "__builtin_decode_uri" => decode_uri_text(&input, true),
        "__builtin_decode_uri_component" => decode_uri_text(&input, false),
        _ => return None,
    };

    Some(host.intern_string(&output))
}

fn encode_uri_text(input: &str, component: bool) -> String {
    let mut out = String::new();
    for byte in input.as_bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
            || (!component
                && matches!(
                    byte,
                    b';' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',' | b'#'
                ));
        if keep {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}

fn decode_uri_text(input: &str, preserve_reserved: bool) -> String {
    let mut out = String::new();
    let mut decoded = Vec::new();
    let bytes = input.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Some(value) = decode_hex_pair(bytes[index + 1], bytes[index + 2]) {
                let reserved = matches!(
                    value,
                    b';' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',' | b'#'
                );
                if preserve_reserved && reserved {
                    flush_decoded_bytes(&mut out, &mut decoded);
                    out.push('%');
                    out.push(bytes[index + 1] as char);
                    out.push(bytes[index + 2] as char);
                } else {
                    decoded.push(value);
                }
                index += 3;
                continue;
            }
        }

        flush_decoded_bytes(&mut out, &mut decoded);
        let ch = input[index..].chars().next().unwrap_or_default();
        out.push(ch);
        index += ch.len_utf8();
    }

    flush_decoded_bytes(&mut out, &mut decoded);
    out
}

fn flush_decoded_bytes(out: &mut String, decoded: &mut Vec<u8>) {
    if decoded.is_empty() {
        return;
    }
    out.push_str(&String::from_utf8_lossy(decoded));
    decoded.clear();
}

fn decode_hex_pair(first: u8, second: u8) -> Option<u8> {
    let first = hex_value(first)?;
    let second = hex_value(second)?;
    Some((first << 4) | second)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
