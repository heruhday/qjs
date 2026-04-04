use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use value::{JSValue, make_number, to_f64};

use crate::{BuiltinHost, BuiltinMethod, install_global_object};

const MATH_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("abs", "__builtin_math_abs"),
    BuiltinMethod::new("ceil", "__builtin_math_ceil"),
    BuiltinMethod::new("floor", "__builtin_math_floor"),
    BuiltinMethod::new("round", "__builtin_math_round"),
    BuiltinMethod::new("trunc", "__builtin_math_trunc"),
    BuiltinMethod::new("sqrt", "__builtin_math_sqrt"),
    BuiltinMethod::new("pow", "__builtin_math_pow"),
    BuiltinMethod::new("max", "__builtin_math_max"),
    BuiltinMethod::new("min", "__builtin_math_min"),
    BuiltinMethod::new("sin", "__builtin_math_sin"),
    BuiltinMethod::new("cos", "__builtin_math_cos"),
    BuiltinMethod::new("tan", "__builtin_math_tan"),
    BuiltinMethod::new("exp", "__builtin_math_exp"),
    BuiltinMethod::new("log", "__builtin_math_log"),
    BuiltinMethod::new("random", "__builtin_math_random"),
];

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let Some(math) = install_global_object(host, global_slots, "Math", MATH_METHODS) else {
        return;
    };

    host.set_property(math, "E", make_number(std::f64::consts::E));
    host.set_property(math, "LN2", make_number(std::f64::consts::LN_2));
    host.set_property(math, "LN10", make_number(std::f64::consts::LN_10));
    host.set_property(math, "LOG2E", make_number(std::f64::consts::LOG2_E));
    host.set_property(math, "LOG10E", make_number(std::f64::consts::LOG10_E));
    host.set_property(math, "PI", make_number(std::f64::consts::PI));
    host.set_property(
        math,
        "SQRT1_2",
        make_number(std::f64::consts::FRAC_1_SQRT_2),
    );
    host.set_property(math, "SQRT2", make_number(std::f64::consts::SQRT_2));
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    Some(match name {
        "__builtin_math_abs" => unary_number(host, args, f64::abs),
        "__builtin_math_ceil" => unary_number(host, args, f64::ceil),
        "__builtin_math_floor" => unary_number(host, args, f64::floor),
        "__builtin_math_round" => unary_number(host, args, f64::round),
        "__builtin_math_trunc" => unary_number(host, args, f64::trunc),
        "__builtin_math_sqrt" => unary_number(host, args, f64::sqrt),
        "__builtin_math_pow" => binary_number(host, args, f64::powf),
        "__builtin_math_max" => fold_numbers(host, args, f64::NEG_INFINITY, f64::max),
        "__builtin_math_min" => fold_numbers(host, args, f64::INFINITY, f64::min),
        "__builtin_math_sin" => unary_number(host, args, f64::sin),
        "__builtin_math_cos" => unary_number(host, args, f64::cos),
        "__builtin_math_tan" => unary_number(host, args, f64::tan),
        "__builtin_math_exp" => unary_number(host, args, f64::exp),
        "__builtin_math_log" => unary_number(host, args, f64::ln),
        "__builtin_math_random" => make_number(pseudo_random()),
        _ => return None,
    })
}

fn first_number<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> f64 {
    args.first()
        .and_then(|&value| to_f64(host.number_value(value)))
        .unwrap_or(f64::NAN)
}

fn unary_number<H: BuiltinHost>(host: &mut H, args: &[JSValue], op: fn(f64) -> f64) -> JSValue {
    make_number(op(first_number(host, args)))
}

fn binary_number<H: BuiltinHost>(
    host: &mut H,
    args: &[JSValue],
    op: fn(f64, f64) -> f64,
) -> JSValue {
    let lhs = args
        .first()
        .and_then(|&value| to_f64(host.number_value(value)))
        .unwrap_or(f64::NAN);
    let rhs = args
        .get(1)
        .and_then(|&value| to_f64(host.number_value(value)))
        .unwrap_or(f64::NAN);
    make_number(op(lhs, rhs))
}

fn fold_numbers<H: BuiltinHost>(
    host: &mut H,
    args: &[JSValue],
    default: f64,
    op: fn(f64, f64) -> f64,
) -> JSValue {
    let value = args
        .iter()
        .filter_map(|&value| to_f64(host.number_value(value)))
        .reduce(op)
        .unwrap_or(default);
    make_number(value)
}

fn pseudo_random() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let mut seed = nanos ^ 0x9E37_79B9_7F4A_7C15;
    seed ^= seed >> 12;
    seed ^= seed << 25;
    seed ^= seed >> 27;
    (seed as f64 / u64::MAX as f64).fract().abs()
}
