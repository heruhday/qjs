use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use value::{JSValue, make_number, to_f64};

use crate::{BuiltinHost, BuiltinMethod, install_global_function, install_methods};

const DATE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("now", "__builtin_date_now"),
    BuiltinMethod::new("parse", "__builtin_date_parse"),
    BuiltinMethod::new("UTC", "__builtin_date_utc"),
];

const DATE_INSTANCE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("getTime", "__builtin_date_get_time"),
    BuiltinMethod::new("valueOf", "__builtin_date_value_of"),
    BuiltinMethod::new("toISOString", "__builtin_date_to_iso_string"),
];

const MILLIS_PROP: &str = "__qjs_date_millis";
const KIND_PROP: &str = "__qjs_builtin_kind";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(host, global_slots, "Date", "__builtin_date", DATE_METHODS);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_date" => Some(host.intern_string(&format_date_string(date_now_millis()))),
        "__builtin_date_now" => Some(make_number(date_now_millis() as f64)),
        "__builtin_date_parse" => Some(date_parse(host, args)),
        "__builtin_date_utc" => Some(date_utc(host, args)),
        "__builtin_date_get_time" | "__builtin_date_value_of" => {
            Some(host.get_property(this_value, MILLIS_PROP))
        }
        "__builtin_date_to_iso_string" => Some(date_to_iso_string(host, this_value)),
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    if name != "__builtin_date" {
        return None;
    }

    let millis = date_constructor_millis(host, args);
    Some(create_date_instance(host, millis))
}

fn create_date_instance<H: BuiltinHost>(host: &mut H, millis: f64) -> JSValue {
    let object = host.create_object();
    let kind = host.intern_string("Date");
    host.set_property(object, KIND_PROP, kind);
    host.set_property(object, MILLIS_PROP, make_number(millis));
    install_methods(host, object, DATE_INSTANCE_METHODS);
    object
}

fn date_now_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => (duration.as_secs_f64() * 1000.0) as i64,
        Err(_) => 0,
    }
}

fn date_parse<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(value) = args.first().copied() else {
        return make_number(f64::NAN);
    };

    let text = host.display_string(value);
    match parse_date_text_to_millis(&text) {
        Some(millis) => make_number(millis as f64),
        None => make_number(f64::NAN),
    }
}

fn date_utc<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let Some(year) = date_integer_arg(host, args, 0, None) else {
        return make_number(f64::NAN);
    };
    let Some(month_index) = date_integer_arg(host, args, 1, None) else {
        return make_number(f64::NAN);
    };
    let Some(day) = date_integer_arg(host, args, 2, Some(1)) else {
        return make_number(f64::NAN);
    };
    let Some(hours) = date_integer_arg(host, args, 3, Some(0)) else {
        return make_number(f64::NAN);
    };
    let Some(minutes) = date_integer_arg(host, args, 4, Some(0)) else {
        return make_number(f64::NAN);
    };
    let Some(seconds) = date_integer_arg(host, args, 5, Some(0)) else {
        return make_number(f64::NAN);
    };
    let Some(milliseconds) = date_integer_arg(host, args, 6, Some(0)) else {
        return make_number(f64::NAN);
    };

    match utc_millis_from_components(
        year,
        month_index,
        day,
        hours,
        minutes,
        seconds,
        milliseconds,
    ) {
        Some(millis) => make_number(millis as f64),
        None => make_number(f64::NAN),
    }
}

fn date_to_iso_string<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    let millis = to_f64(host.get_property(this_value, MILLIS_PROP)).unwrap_or(f64::NAN);
    if !millis.is_finite() {
        return host.intern_string("Invalid Date");
    }
    host.intern_string(&format_date_string(millis as i64))
}

fn date_constructor_millis<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> f64 {
    match args {
        [] => date_now_millis() as f64,
        [value] => {
            if let Some(text) = host.string_text(*value) {
                parse_date_text_to_millis(text)
                    .map(|millis| millis as f64)
                    .unwrap_or(f64::NAN)
            } else {
                to_f64(host.number_value(*value)).unwrap_or(f64::NAN)
            }
        }
        _ => {
            let Some(year) = date_integer_arg(host, args, 0, None) else {
                return f64::NAN;
            };
            let Some(month_index) = date_integer_arg(host, args, 1, None) else {
                return f64::NAN;
            };
            let Some(day) = date_integer_arg(host, args, 2, Some(1)) else {
                return f64::NAN;
            };
            let Some(hours) = date_integer_arg(host, args, 3, Some(0)) else {
                return f64::NAN;
            };
            let Some(minutes) = date_integer_arg(host, args, 4, Some(0)) else {
                return f64::NAN;
            };
            let Some(seconds) = date_integer_arg(host, args, 5, Some(0)) else {
                return f64::NAN;
            };
            let Some(milliseconds) = date_integer_arg(host, args, 6, Some(0)) else {
                return f64::NAN;
            };

            utc_millis_from_components(
                year,
                month_index,
                day,
                hours,
                minutes,
                seconds,
                milliseconds,
            )
            .map(|millis| millis as f64)
            .unwrap_or(f64::NAN)
        }
    }
}

fn date_integer_arg<H: BuiltinHost>(
    host: &mut H,
    args: &[JSValue],
    index: usize,
    default: Option<i64>,
) -> Option<i64> {
    let value = match args.get(index).copied() {
        Some(value) => value,
        None => return default,
    };

    let numeric = to_f64(host.number_value(value))?;
    if !numeric.is_finite() || numeric < i64::MIN as f64 || numeric > i64::MAX as f64 {
        return None;
    }

    Some(numeric.trunc() as i64)
}

fn parse_date_text_to_millis(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Some(value.timestamp_millis());
    }

    if let Ok(value) = DateTime::parse_from_rfc2822(text) {
        return Some(value.timestamp_millis());
    }

    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(value) = NaiveDateTime::parse_from_str(text, format) {
            return Some(value.and_utc().timestamp_millis());
        }
    }

    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(0, 0, 0)
        .map(|value| value.and_utc().timestamp_millis())
}

fn utc_millis_from_components(
    year: i64,
    month_index: i64,
    day: i64,
    hours: i64,
    minutes: i64,
    seconds: i64,
    milliseconds: i64,
) -> Option<i64> {
    let year = if (0..=99).contains(&year) {
        year + 1900
    } else {
        year
    };

    let total_months = year.checked_mul(12)?.checked_add(month_index)?;
    let normalized_year = i32::try_from(total_months.div_euclid(12)).ok()?;
    let normalized_month = u32::try_from(total_months.rem_euclid(12) + 1).ok()?;

    let base = NaiveDate::from_ymd_opt(normalized_year, normalized_month, 1)?
        .and_hms_milli_opt(0, 0, 0, 0)?;

    let value = base
        .checked_add_signed(Duration::days(day - 1))?
        .checked_add_signed(Duration::hours(hours))?
        .checked_add_signed(Duration::minutes(minutes))?
        .checked_add_signed(Duration::seconds(seconds))?
        .checked_add_signed(Duration::milliseconds(milliseconds))?;

    Some(value.and_utc().timestamp_millis())
}

fn format_date_string(millis: i64) -> String {
    Utc.timestamp_millis_opt(millis)
        .single()
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| "Invalid Date".to_owned())
}
