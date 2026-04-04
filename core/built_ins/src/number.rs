use std::collections::HashMap;

use value::{JSValue, make_bool, make_number, to_f64};

use crate::{BuiltinHost, BuiltinMethod, install_global_function};

const NUMBER_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("isFinite", "__builtin_number_is_finite"),
    BuiltinMethod::new("isNaN", "__builtin_number_is_nan"),
    BuiltinMethod::new("isInteger", "__builtin_number_is_integer"),
    BuiltinMethod::new("parseFloat", "__builtin_number_parse_float"),
    BuiltinMethod::new("parseInt", "__builtin_number_parse_int"),
];

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(
        host,
        global_slots,
        "Number",
        "__builtin_number",
        NUMBER_METHODS,
    );
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_number" => Some(number_constructor(host, args)),
        "__builtin_number_to_fixed" => Some(number_to_fixed(host, this_value, args)),
        "__builtin_number_is_finite" => Some(number_is_finite(host, args)),
        "__builtin_number_is_nan" => Some(number_is_nan(host, args)),
        "__builtin_number_is_integer" => Some(number_is_integer(host, args)),
        "__builtin_number_parse_float" => Some(number_parse_float(host, args)),
        "__builtin_number_parse_int" => Some(number_parse_int(host, args)),
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    (name == "__builtin_number").then(|| number_constructor(host, args))
}

fn number_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    args.first()
        .copied()
        .map(|value| host.number_value(value))
        .unwrap_or_else(|| make_number(0.0))
}

fn number_to_fixed<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let digits = args
        .first()
        .and_then(|&value| to_f64(host.number_value(value)))
        .map(|value| value.clamp(0.0, 100.0).trunc() as usize)
        .unwrap_or(0);

    let number = to_f64(host.number_value(this_value)).unwrap_or(f64::NAN);
    let rendered = if number.is_nan() {
        "NaN".to_owned()
    } else if number.is_infinite() && number.is_sign_positive() {
        "Infinity".to_owned()
    } else if number.is_infinite() {
        "-Infinity".to_owned()
    } else {
        format!("{number:.digits$}")
    };

    host.intern_string(&rendered)
}

fn number_is_finite<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let value = args
        .first()
        .copied()
        .and_then(|value| to_f64(host.number_value(value)));
    make_bool(value.is_some_and(f64::is_finite))
}

fn number_is_nan<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let value = args
        .first()
        .copied()
        .and_then(|value| to_f64(host.number_value(value)));
    make_bool(value.is_some_and(f64::is_nan))
}

fn number_is_integer<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let value = args
        .first()
        .copied()
        .and_then(|value| to_f64(host.number_value(value)));
    make_bool(value.is_some_and(|number| number.is_finite() && number.fract() == 0.0))
}

fn number_parse_float<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(text) = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
    else {
        return make_number(f64::NAN);
    };
    make_number(text.trim().parse::<f64>().unwrap_or(f64::NAN))
}

fn number_parse_int<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(text) = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
    else {
        return make_number(f64::NAN);
    };
    let radix = args
        .get(1)
        .and_then(|&value| to_f64(host.number_value(value)))
        .map(|value| value.trunc() as u32)
        .filter(|radix| (2..=36).contains(radix))
        .unwrap_or(10);
    let trimmed = text.trim();
    let negative = trimmed.starts_with('-');
    let digits = trimmed.trim_start_matches(['+', '-']);
    match i64::from_str_radix(digits, radix) {
        Ok(value) => make_number(if negative {
            -(value as f64)
        } else {
            value as f64
        }),
        Err(_) => make_number(f64::NAN),
    }
}
