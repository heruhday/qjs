use value::{JSValue, to_f64};

use crate::BuiltinHost;

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_number_to_fixed" => Some(number_to_fixed(host, this_value, args)),
        _ => None,
    }
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
