use std::collections::HashMap;

use chrono::{TimeZone, Utc};
use value::{JSValue, make_number, make_undefined, to_f64};

use crate::{
    BuiltinHost, BuiltinMethod, create_array_from_values, create_builtin_callable,
    install_global_object, install_methods,
};

const INTL_METHODS: &[BuiltinMethod] = &[BuiltinMethod::new(
    "getCanonicalLocales",
    "__builtin_intl_get_canonical_locales",
)];

const LOCALE_METHODS: &[BuiltinMethod] = &[BuiltinMethod::new(
    "toString",
    "__builtin_intl_locale_to_string",
)];
const DATE_TIME_FORMAT_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("format", "__builtin_intl_datetime_format_format"),
    BuiltinMethod::new(
        "resolvedOptions",
        "__builtin_intl_datetime_format_resolved_options",
    ),
];
const NUMBER_FORMAT_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("format", "__builtin_intl_number_format_format"),
    BuiltinMethod::new(
        "resolvedOptions",
        "__builtin_intl_number_format_resolved_options",
    ),
];

const KIND_PROP: &str = "__qjs_builtin_kind";
const LOCALE_PROP: &str = "__qjs_intl_locale";
const BASE_NAME_PROP: &str = "__qjs_intl_base_name";
const MIN_FRACTION_PROP: &str = "__qjs_intl_min_fraction";
const MAX_FRACTION_PROP: &str = "__qjs_intl_max_fraction";
const DATE_MILLIS_PROP: &str = "__qjs_date_millis";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let Some(intl) = install_global_object(host, global_slots, "Intl", INTL_METHODS) else {
        return;
    };

    let locale = create_builtin_callable(host, "__builtin_intl_locale");
    let date_time_format = create_builtin_callable(host, "__builtin_intl_datetime_format");
    let number_format = create_builtin_callable(host, "__builtin_intl_number_format");
    host.set_property(intl, "Locale", locale);
    host.set_property(intl, "DateTimeFormat", date_time_format);
    host.set_property(intl, "NumberFormat", number_format);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    Some(match name {
        "__builtin_intl_get_canonical_locales" => intl_get_canonical_locales(host, args),
        "__builtin_intl_locale" => locale_constructor(host, args),
        "__builtin_intl_locale_to_string" => locale_to_string(host, this_value),
        "__builtin_intl_datetime_format" => datetime_format_constructor(host, args),
        "__builtin_intl_datetime_format_format" => datetime_format(host, this_value, args),
        "__builtin_intl_datetime_format_resolved_options" => {
            datetime_format_resolved_options(host, this_value)
        }
        "__builtin_intl_number_format" => number_format_constructor(host, args),
        "__builtin_intl_number_format_format" => number_format(host, this_value, args),
        "__builtin_intl_number_format_resolved_options" => {
            number_format_resolved_options(host, this_value)
        }
        _ => return None,
    })
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_intl_locale" => Some(locale_constructor(host, args)),
        "__builtin_intl_datetime_format" => Some(datetime_format_constructor(host, args)),
        "__builtin_intl_number_format" => Some(number_format_constructor(host, args)),
        _ => None,
    }
}

fn intl_get_canonical_locales<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let values = args
        .first()
        .copied()
        .and_then(|value| host.array_values(value))
        .unwrap_or_else(|| {
            args.first()
                .copied()
                .map(|value| vec![value])
                .unwrap_or_default()
        });

    let canonical_text = values
        .into_iter()
        .map(|value| canonicalize_locale(&host.display_string(value)))
        .collect::<Vec<_>>();
    let canonical = canonical_text
        .into_iter()
        .map(|value| host.intern_string(&value))
        .collect::<Vec<_>>();
    create_array_from_values(host, canonical)
}

fn locale_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let locale = canonicalize_locale(
        &args
            .first()
            .copied()
            .map(|value| host.display_string(value))
            .unwrap_or_else(|| "und".to_owned()),
    );
    let out = host.create_object();
    let kind = host.intern_string("Intl.Locale");
    let locale_value = host.intern_string(&locale);
    host.set_property(out, KIND_PROP, kind);
    host.set_property(out, BASE_NAME_PROP, locale_value);
    host.set_property(out, "baseName", locale_value);
    install_methods(host, out, LOCALE_METHODS);
    out
}

fn locale_to_string<H: BuiltinHost>(host: &mut H, locale: JSValue) -> JSValue {
    host.get_property(locale, BASE_NAME_PROP)
}

fn datetime_format_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let locale = canonicalize_locale(
        &args
            .first()
            .copied()
            .map(|value| host.display_string(value))
            .unwrap_or_else(|| "en-US".to_owned()),
    );
    let out = host.create_object();
    let kind = host.intern_string("Intl.DateTimeFormat");
    let locale_value = host.intern_string(&locale);
    host.set_property(out, KIND_PROP, kind);
    host.set_property(out, LOCALE_PROP, locale_value);
    install_methods(host, out, DATE_TIME_FORMAT_METHODS);
    out
}

fn datetime_format<H: BuiltinHost>(host: &mut H, format: JSValue, args: &[JSValue]) -> JSValue {
    let millis = args
        .first()
        .copied()
        .map(|value| date_like_millis(host, value))
        .unwrap_or(0);
    let rendered = Utc
        .timestamp_millis_opt(millis)
        .single()
        .map(|value| value.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "Invalid Date".to_owned());
    let _ = format;
    host.intern_string(&rendered)
}

fn datetime_format_resolved_options<H: BuiltinHost>(host: &mut H, format: JSValue) -> JSValue {
    let out = host.create_object();
    let locale = host.get_property(format, LOCALE_PROP);
    host.set_property(out, "locale", locale);
    out
}

fn number_format_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let locale = canonicalize_locale(
        &args
            .first()
            .copied()
            .map(|value| host.display_string(value))
            .unwrap_or_else(|| "en-US".to_owned()),
    );
    let options = args.get(1).copied().unwrap_or_else(make_undefined);
    let min_fraction = option_fraction_digits(host, options, "minimumFractionDigits", 0);
    let max_fraction =
        option_fraction_digits(host, options, "maximumFractionDigits", 3).max(min_fraction);

    let out = host.create_object();
    let kind = host.intern_string("Intl.NumberFormat");
    let locale_value = host.intern_string(&locale);
    host.set_property(out, KIND_PROP, kind);
    host.set_property(out, LOCALE_PROP, locale_value);
    host.set_property(out, MIN_FRACTION_PROP, make_number(min_fraction as f64));
    host.set_property(out, MAX_FRACTION_PROP, make_number(max_fraction as f64));
    install_methods(host, out, NUMBER_FORMAT_METHODS);
    out
}

fn number_format<H: BuiltinHost>(host: &mut H, format: JSValue, args: &[JSValue]) -> JSValue {
    let number = args
        .first()
        .and_then(|&value| to_f64(host.number_value(value)))
        .unwrap_or(f64::NAN);
    let min_fraction = number_fraction_value(host, format, MIN_FRACTION_PROP);
    let max_fraction = number_fraction_value(host, format, MAX_FRACTION_PROP).max(min_fraction);
    host.intern_string(&render_number(number, min_fraction, max_fraction))
}

fn number_format_resolved_options<H: BuiltinHost>(host: &mut H, format: JSValue) -> JSValue {
    let out = host.create_object();
    let locale = host.get_property(format, LOCALE_PROP);
    let min_fraction = host.get_property(format, MIN_FRACTION_PROP);
    let max_fraction = host.get_property(format, MAX_FRACTION_PROP);
    host.set_property(out, "locale", locale);
    host.set_property(out, "minimumFractionDigits", min_fraction);
    host.set_property(out, "maximumFractionDigits", max_fraction);
    out
}

fn canonicalize_locale(input: &str) -> String {
    let mut parts = input
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return "und".to_owned();
    }

    for (index, part) in parts.iter_mut().enumerate() {
        if index == 0 {
            *part = part.to_ascii_lowercase();
        } else if part.len() == 2 {
            *part = part.to_ascii_uppercase();
        } else if part.len() == 4 {
            let mut chars = part.chars();
            let first = chars.next().unwrap_or_default().to_ascii_uppercase();
            let rest = chars.as_str().to_ascii_lowercase();
            *part = format!("{first}{rest}");
        }
    }

    parts.join("-")
}

fn option_fraction_digits<H: BuiltinHost>(
    host: &mut H,
    options: JSValue,
    name: &str,
    default: usize,
) -> usize {
    to_f64(host.number_value(host.get_property(options, name)))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.trunc() as usize)
        .unwrap_or(default)
}

fn number_fraction_value<H: BuiltinHost>(host: &mut H, format: JSValue, name: &str) -> usize {
    to_f64(host.number_value(host.get_property(format, name)))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.trunc() as usize)
        .unwrap_or(0)
}

fn render_number(number: f64, min_fraction: usize, max_fraction: usize) -> String {
    if number.is_nan() {
        return "NaN".to_owned();
    }
    if number.is_infinite() {
        return if number.is_sign_positive() {
            "Infinity".to_owned()
        } else {
            "-Infinity".to_owned()
        };
    }

    let mut rendered = format!("{number:.max_fraction$}");
    if let Some(dot) = rendered.find('.') {
        while rendered.ends_with('0') && rendered.len().saturating_sub(dot + 1) > min_fraction {
            rendered.pop();
        }
        if rendered.ends_with('.') {
            rendered.pop();
        }
    }
    rendered
}

fn date_like_millis<H: BuiltinHost>(host: &mut H, value: JSValue) -> i64 {
    if let Some(millis) = to_f64(host.number_value(host.get_property(value, DATE_MILLIS_PROP))) {
        return millis as i64;
    }
    to_f64(host.number_value(value)).unwrap_or(0.0) as i64
}
