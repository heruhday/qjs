use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime};
use value::{JSValue, make_number, to_f64};

use crate::{BuiltinHost, BuiltinMethod, BuiltinObject};

const DATE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("now", "__builtin_date_now"),
    BuiltinMethod::new("parse", "__builtin_date_parse"),
    BuiltinMethod::new("UTC", "__builtin_date_utc"),
];

pub(crate) const OBJECTS: &[BuiltinObject] = &[BuiltinObject::new("Date", DATE_METHODS)];

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_date_now" => Some(date_now(args)),
        "__builtin_date_parse" => Some(date_parse(host, args)),
        "__builtin_date_utc" => Some(date_utc(host, args)),
        _ => None,
    }
}

fn date_now(_args: &[JSValue]) -> JSValue {
    let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs_f64() * 1000.0,
        Err(_) => 0.0,
    };

    make_number(millis)
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
