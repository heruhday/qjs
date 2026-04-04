use std::collections::HashMap;

use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use value::{JSValue, make_number, to_f64};

use crate::{
    BuiltinHost, BuiltinMethod, create_builtin_callable, install_global_object, install_methods,
};

const TEMPORAL_METHODS: &[BuiltinMethod] = &[];
const NOW_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("instant", "__builtin_temporal_now_instant"),
    BuiltinMethod::new("plainDateISO", "__builtin_temporal_now_plain_date_iso"),
    BuiltinMethod::new(
        "plainDateTimeISO",
        "__builtin_temporal_now_plain_datetime_iso",
    ),
];
const INSTANT_METHODS: &[BuiltinMethod] = &[BuiltinMethod::new(
    "toString",
    "__builtin_temporal_instant_to_string",
)];
const PLAIN_DATE_METHODS: &[BuiltinMethod] = &[BuiltinMethod::new(
    "toString",
    "__builtin_temporal_plain_date_to_string",
)];
const PLAIN_DATE_TIME_METHODS: &[BuiltinMethod] = &[BuiltinMethod::new(
    "toString",
    "__builtin_temporal_plain_datetime_to_string",
)];

const KIND_PROP: &str = "__qjs_builtin_kind";
const MILLIS_PROP: &str = "__qjs_temporal_millis";
const DATE_TEXT_PROP: &str = "__qjs_temporal_date_text";
const DATETIME_TEXT_PROP: &str = "__qjs_temporal_datetime_text";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let Some(temporal) = install_global_object(host, global_slots, "Temporal", TEMPORAL_METHODS)
    else {
        return;
    };

    let now = host.create_object();
    install_methods(host, now, NOW_METHODS);
    let instant_ctor = create_builtin_callable(host, "__builtin_temporal_instant");
    let plain_date_ctor = create_builtin_callable(host, "__builtin_temporal_plain_date");
    let plain_datetime_ctor = create_builtin_callable(host, "__builtin_temporal_plain_datetime");
    host.set_property(temporal, "Now", now);
    host.set_property(temporal, "Instant", instant_ctor);
    host.set_property(temporal, "PlainDate", plain_date_ctor);
    host.set_property(temporal, "PlainDateTime", plain_datetime_ctor);

    let instant = host.get_property(temporal, "Instant");
    let instant_from_epoch =
        create_builtin_callable(host, "__builtin_temporal_instant_from_epoch_milliseconds");
    host.set_property(instant, "fromEpochMilliseconds", instant_from_epoch);

    let plain_date = host.get_property(temporal, "PlainDate");
    let plain_date_from = create_builtin_callable(host, "__builtin_temporal_plain_date_from");
    host.set_property(plain_date, "from", plain_date_from);

    let plain_datetime = host.get_property(temporal, "PlainDateTime");
    let plain_datetime_from =
        create_builtin_callable(host, "__builtin_temporal_plain_datetime_from");
    host.set_property(plain_datetime, "from", plain_datetime_from);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    Some(match name {
        "__builtin_temporal_instant" | "__builtin_temporal_instant_from_epoch_milliseconds" => {
            instant_constructor(host, args)
        }
        "__builtin_temporal_instant_to_string" => instant_to_string(host, this_value),
        "__builtin_temporal_plain_date" | "__builtin_temporal_plain_date_from" => {
            plain_date_constructor(host, args)
        }
        "__builtin_temporal_plain_date_to_string" => plain_date_to_string(host, this_value),
        "__builtin_temporal_plain_datetime" | "__builtin_temporal_plain_datetime_from" => {
            plain_datetime_constructor(host, args)
        }
        "__builtin_temporal_plain_datetime_to_string" => plain_datetime_to_string(host, this_value),
        "__builtin_temporal_now_instant" => {
            instant_from_millis(host, Utc::now().timestamp_millis())
        }
        "__builtin_temporal_now_plain_date_iso" => {
            plain_date_from_date(host, Utc::now().date_naive())
        }
        "__builtin_temporal_now_plain_datetime_iso" => {
            plain_datetime_from_naive(host, Utc::now().naive_utc())
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
        "__builtin_temporal_instant" => Some(instant_constructor(host, args)),
        "__builtin_temporal_plain_date" => Some(plain_date_constructor(host, args)),
        "__builtin_temporal_plain_datetime" => Some(plain_datetime_constructor(host, args)),
        _ => None,
    }
}

fn instant_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let millis = args
        .first()
        .and_then(|&value| to_f64(host.number_value(value)))
        .unwrap_or(0.0) as i64;
    instant_from_millis(host, millis)
}

fn instant_from_millis<H: BuiltinHost>(host: &mut H, millis: i64) -> JSValue {
    let instant = host.create_object();
    let kind = host.intern_string("Temporal.Instant");
    host.set_property(instant, KIND_PROP, kind);
    host.set_property(instant, MILLIS_PROP, make_number(millis as f64));
    host.set_property(instant, "epochMilliseconds", make_number(millis as f64));
    install_methods(host, instant, INSTANT_METHODS);
    instant
}

fn instant_to_string<H: BuiltinHost>(host: &mut H, instant: JSValue) -> JSValue {
    let millis =
        to_f64(host.number_value(host.get_property(instant, MILLIS_PROP))).unwrap_or(0.0) as i64;
    let rendered = Utc
        .timestamp_millis_opt(millis)
        .single()
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| "Invalid Temporal.Instant".to_owned());
    host.intern_string(&rendered)
}

fn plain_date_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    if let Some(text) = args
        .first()
        .and_then(|&value| host.string_text(value))
        .and_then(|text| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok())
    {
        return plain_date_from_date(host, text);
    }

    let year = numeric_component(host, args, 0, 1970);
    let month = numeric_component(host, args, 1, 1) as u32;
    let day = numeric_component(host, args, 2, 1) as u32;
    let date = NaiveDate::from_ymd_opt(year as i32, month, day)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid fallback date"));
    plain_date_from_date(host, date)
}

fn plain_date_from_date<H: BuiltinHost>(host: &mut H, date: NaiveDate) -> JSValue {
    let out = host.create_object();
    let rendered = format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day());
    let kind = host.intern_string("Temporal.PlainDate");
    let rendered_value = host.intern_string(&rendered);
    host.set_property(out, KIND_PROP, kind);
    host.set_property(out, DATE_TEXT_PROP, rendered_value);
    host.set_property(out, "year", make_number(date.year() as f64));
    host.set_property(out, "month", make_number(date.month() as f64));
    host.set_property(out, "day", make_number(date.day() as f64));
    install_methods(host, out, PLAIN_DATE_METHODS);
    out
}

fn plain_date_to_string<H: BuiltinHost>(host: &mut H, date: JSValue) -> JSValue {
    host.get_property(date, DATE_TEXT_PROP)
}

fn plain_datetime_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    if let Some(text) = args
        .first()
        .and_then(|&value| host.string_text(value))
        .and_then(parse_plain_datetime)
    {
        return plain_datetime_from_naive(host, text);
    }

    let year = numeric_component(host, args, 0, 1970);
    let month = numeric_component(host, args, 1, 1) as u32;
    let day = numeric_component(host, args, 2, 1) as u32;
    let hour = numeric_component(host, args, 3, 0) as u32;
    let minute = numeric_component(host, args, 4, 0) as u32;
    let second = numeric_component(host, args, 5, 0) as u32;
    let millisecond = numeric_component(host, args, 6, 0) as u32;

    let datetime = NaiveDate::from_ymd_opt(year as i32, month, day)
        .and_then(|date| date.and_hms_milli_opt(hour, minute, second, millisecond))
        .unwrap_or_else(|| {
            NaiveDate::from_ymd_opt(1970, 1, 1)
                .and_then(|date| date.and_hms_milli_opt(0, 0, 0, 0))
                .expect("valid fallback datetime")
        });
    plain_datetime_from_naive(host, datetime)
}

fn plain_datetime_from_naive<H: BuiltinHost>(host: &mut H, datetime: NaiveDateTime) -> JSValue {
    let out = host.create_object();
    let rendered = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}",
        datetime.year(),
        datetime.month(),
        datetime.day(),
        datetime.hour(),
        datetime.minute(),
        datetime.second(),
        datetime.and_utc().timestamp_subsec_millis()
    );
    let kind = host.intern_string("Temporal.PlainDateTime");
    let rendered_value = host.intern_string(&rendered);
    host.set_property(out, KIND_PROP, kind);
    host.set_property(out, DATETIME_TEXT_PROP, rendered_value);
    host.set_property(out, "year", make_number(datetime.year() as f64));
    host.set_property(out, "month", make_number(datetime.month() as f64));
    host.set_property(out, "day", make_number(datetime.day() as f64));
    host.set_property(out, "hour", make_number(datetime.hour() as f64));
    host.set_property(out, "minute", make_number(datetime.minute() as f64));
    host.set_property(out, "second", make_number(datetime.second() as f64));
    install_methods(host, out, PLAIN_DATE_TIME_METHODS);
    out
}

fn plain_datetime_to_string<H: BuiltinHost>(host: &mut H, datetime: JSValue) -> JSValue {
    host.get_property(datetime, DATETIME_TEXT_PROP)
}

fn parse_plain_datetime(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").ok())
}

fn numeric_component<H: BuiltinHost>(
    host: &mut H,
    args: &[JSValue],
    index: usize,
    default: i64,
) -> i64 {
    args.get(index)
        .and_then(|&value| to_f64(host.number_value(value)))
        .filter(|value| value.is_finite())
        .map(|value| value.trunc() as i64)
        .unwrap_or(default)
}
