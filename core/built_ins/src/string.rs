use std::collections::HashMap;

use value::{JSValue, make_bool, make_number, make_undefined, to_f64};

use crate::{
    BuiltinHost, BuiltinMethod, create_array_from_values, install_global_function, install_methods,
};

const STRING_STATIC_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("fromCharCode", "__builtin_string_from_char_code"),
    BuiltinMethod::new("fromCodePoint", "__builtin_string_from_code_point"),
];

const STRING_INSTANCE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("at", "__builtin_string_at"),
    BuiltinMethod::new("charAt", "__builtin_string_char_at"),
    BuiltinMethod::new("charCodeAt", "__builtin_string_char_code_at"),
    BuiltinMethod::new("codePointAt", "__builtin_string_code_point_at"),
    BuiltinMethod::new("concat", "__builtin_string_concat"),
    BuiltinMethod::new("endsWith", "__builtin_string_ends_with"),
    BuiltinMethod::new("includes", "__builtin_string_includes"),
    BuiltinMethod::new("indexOf", "__builtin_string_index_of"),
    BuiltinMethod::new("lastIndexOf", "__builtin_string_last_index_of"),
    BuiltinMethod::new("padEnd", "__builtin_string_pad_end"),
    BuiltinMethod::new("padStart", "__builtin_string_pad_start"),
    BuiltinMethod::new("repeat", "__builtin_string_repeat"),
    BuiltinMethod::new("replace", "__builtin_string_replace"),
    BuiltinMethod::new("replaceAll", "__builtin_string_replace_all"),
    BuiltinMethod::new("search", "__builtin_string_search"),
    BuiltinMethod::new("slice", "__builtin_string_slice"),
    BuiltinMethod::new("split", "__builtin_string_split"),
    BuiltinMethod::new("startsWith", "__builtin_string_starts_with"),
    BuiltinMethod::new("substring", "__builtin_string_substring"),
    BuiltinMethod::new("toLowerCase", "__builtin_string_to_lower_case"),
    BuiltinMethod::new("toString", "__builtin_string_to_string"),
    BuiltinMethod::new("toUpperCase", "__builtin_string_to_upper_case"),
    BuiltinMethod::new("trim", "__builtin_string_trim"),
    BuiltinMethod::new("trimEnd", "__builtin_string_trim_end"),
    BuiltinMethod::new("trimStart", "__builtin_string_trim_start"),
    BuiltinMethod::new("valueOf", "__builtin_string_value_of"),
];

#[cfg(test)]
const STRING_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("fromCharCode", "__builtin_string_from_char_code"),
    BuiltinMethod::new("fromCodePoint", "__builtin_string_from_code_point"),
    BuiltinMethod::new("at", "__builtin_string_at"),
    BuiltinMethod::new("charAt", "__builtin_string_char_at"),
    BuiltinMethod::new("charCodeAt", "__builtin_string_char_code_at"),
    BuiltinMethod::new("codePointAt", "__builtin_string_code_point_at"),
    BuiltinMethod::new("concat", "__builtin_string_concat"),
    BuiltinMethod::new("endsWith", "__builtin_string_ends_with"),
    BuiltinMethod::new("includes", "__builtin_string_includes"),
    BuiltinMethod::new("indexOf", "__builtin_string_index_of"),
    BuiltinMethod::new("lastIndexOf", "__builtin_string_last_index_of"),
    BuiltinMethod::new("padEnd", "__builtin_string_pad_end"),
    BuiltinMethod::new("padStart", "__builtin_string_pad_start"),
    BuiltinMethod::new("repeat", "__builtin_string_repeat"),
    BuiltinMethod::new("replace", "__builtin_string_replace"),
    BuiltinMethod::new("replaceAll", "__builtin_string_replace_all"),
    BuiltinMethod::new("search", "__builtin_string_search"),
    BuiltinMethod::new("slice", "__builtin_string_slice"),
    BuiltinMethod::new("split", "__builtin_string_split"),
    BuiltinMethod::new("startsWith", "__builtin_string_starts_with"),
    BuiltinMethod::new("substring", "__builtin_string_substring"),
    BuiltinMethod::new("toLowerCase", "__builtin_string_to_lower_case"),
    BuiltinMethod::new("toString", "__builtin_string_to_string"),
    BuiltinMethod::new("toUpperCase", "__builtin_string_to_upper_case"),
    BuiltinMethod::new("trim", "__builtin_string_trim"),
    BuiltinMethod::new("trimEnd", "__builtin_string_trim_end"),
    BuiltinMethod::new("trimStart", "__builtin_string_trim_start"),
    BuiltinMethod::new("valueOf", "__builtin_string_value_of"),
];

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let Some(constructor) = install_global_function(
        host,
        global_slots,
        "String",
        "__builtin_string",
        STRING_STATIC_METHODS,
    ) else {
        return;
    };

    host.set_property(constructor, "length", make_number(1.0));
    let prototype = create_string_prototype(host);
    host.set_property(constructor, "prototype", prototype);
}

pub fn create_string_prototype<H: BuiltinHost>(host: &mut H) -> JSValue {
    let prototype = host.create_object();
    crate::object::attach_object_methods(host, prototype);
    install_methods(host, prototype, STRING_INSTANCE_METHODS);
    prototype
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    Some(match name {
        "__builtin_string" => string_constructor(host, args),
        "__builtin_string_from_char_code" => string_from_char_code(host, args),
        "__builtin_string_from_code_point" => string_from_code_point(host, args),
        "__builtin_string_at" => string_at(host, this_value, args),
        "__builtin_string_char_at" => string_char_at(host, this_value, args),
        "__builtin_string_char_code_at" => string_char_code_at(host, this_value, args),
        "__builtin_string_code_point_at" => string_code_point_at(host, this_value, args),
        "__builtin_string_concat" => string_concat(host, this_value, args),
        "__builtin_string_ends_with" => string_ends_with(host, this_value, args),
        "__builtin_string_includes" => string_includes(host, this_value, args),
        "__builtin_string_index_of" => string_index_of_method(host, this_value, args),
        "__builtin_string_last_index_of" => string_last_index_of_method(host, this_value, args),
        "__builtin_string_pad_end" => string_pad_end(host, this_value, args),
        "__builtin_string_pad_start" => string_pad_start(host, this_value, args),
        "__builtin_string_repeat" => string_repeat(host, this_value, args),
        "__builtin_string_replace" => string_replace(host, this_value, args),
        "__builtin_string_replace_all" => string_replace_all(host, this_value, args),
        "__builtin_string_search" => string_search(host, this_value, args),
        "__builtin_string_slice" => string_slice(host, this_value, args),
        "__builtin_string_split" => string_split(host, this_value, args),
        "__builtin_string_starts_with" => string_starts_with(host, this_value, args),
        "__builtin_string_substring" => string_substring(host, this_value, args),
        "__builtin_string_to_lower_case" => string_to_lower_case(host, this_value),
        "__builtin_string_to_string" => string_to_string(host, this_value),
        "__builtin_string_to_upper_case" => string_to_upper_case(host, this_value),
        "__builtin_string_trim" => string_trim(host, this_value),
        "__builtin_string_trim_end" => string_trim_end(host, this_value),
        "__builtin_string_trim_start" => string_trim_start(host, this_value),
        "__builtin_string_value_of" => string_to_string(host, this_value),
        _ => return None,
    })
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    (name == "__builtin_string").then(|| string_constructor(host, args))
}

fn string_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let text = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_default();
    host.intern_string(&text)
}

fn string_from_char_code<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let rendered = args
        .iter()
        .filter_map(|&value| to_f64(host.number_value(value)))
        .map(|value| {
            let code = (value.trunc() as i64).rem_euclid(1 << 16) as u32;
            char::from_u32(code).unwrap_or('\u{FFFD}')
        })
        .collect::<String>();

    host.intern_string(&rendered)
}

fn string_from_code_point<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let mut rendered = String::new();
    for &value in args {
        let Some(number) = to_f64(host.number_value(value)) else {
            return host.intern_string("RangeError: code point must be numeric");
        };
        if !number.is_finite() || number.fract() != 0.0 {
            return host.intern_string("RangeError: code point must be a finite integer");
        }
        if !(0.0..=0x10FFFF as f64).contains(&number) {
            return host.intern_string("RangeError: code point out of range");
        }

        let Some(ch) = char::from_u32(number as u32) else {
            return host.intern_string("RangeError: code point out of range");
        };
        rendered.push(ch);
    }
    host.intern_string(&rendered)
}

fn string_value<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> String {
    host.display_string(this_value)
}

fn string_len(text: &str) -> usize {
    text.chars().count()
}

fn to_integer_or_infinity<H: BuiltinHost>(host: &mut H, value: JSValue) -> f64 {
    match to_f64(host.number_value(value)) {
        Some(number) if number.is_finite() => number.trunc(),
        Some(number) => number,
        None => 0.0,
    }
}

fn relative_index<H: BuiltinHost>(
    host: &mut H,
    value: Option<JSValue>,
    len: usize,
    default: usize,
) -> usize {
    let Some(value) = value else {
        return default;
    };
    let integer = to_integer_or_infinity(host, value);
    if integer.is_sign_negative() {
        ((len as f64) + integer).max(0.0) as usize
    } else if integer.is_infinite() {
        len
    } else {
        integer.min(len as f64) as usize
    }
}

fn non_negative_limit<H: BuiltinHost>(
    host: &mut H,
    value: Option<JSValue>,
    default: usize,
) -> usize {
    let Some(value) = value else {
        return default;
    };
    let Some(number) = to_f64(host.number_value(value)) else {
        return 0;
    };
    if number.is_nan() || number <= 0.0 {
        0
    } else if number.is_infinite() {
        u32::MAX as usize
    } else {
        number.trunc().min(u32::MAX as f64) as usize
    }
}

fn char_at(text: &str, index: usize) -> Option<String> {
    text.chars().nth(index).map(|ch| ch.to_string())
}

fn slice_chars(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

fn string_index_of(text: &str, search: &str, start: usize) -> Option<usize> {
    let haystack = text.chars().collect::<Vec<_>>();
    let needle = search.chars().collect::<Vec<_>>();
    if needle.is_empty() {
        return Some(start.min(haystack.len()));
    }
    if start > haystack.len() || needle.len() > haystack.len().saturating_sub(start) {
        return None;
    }
    (start..=haystack.len() - needle.len())
        .find(|&index| haystack[index..index + needle.len()] == needle[..])
}

fn string_last_index_of(text: &str, search: &str, start: usize) -> Option<usize> {
    let haystack = text.chars().collect::<Vec<_>>();
    let needle = search.chars().collect::<Vec<_>>();
    if needle.is_empty() {
        return Some(start.min(haystack.len()));
    }
    if needle.len() > haystack.len() {
        return None;
    }
    let max_start = haystack.len().saturating_sub(needle.len());
    (0..=start.min(max_start))
        .rev()
        .find(|&index| haystack[index..index + needle.len()] == needle[..])
}

fn string_to_string<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    let text = string_value(host, this_value);
    host.intern_string(&text)
}

fn string_at<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let text = string_value(host, this_value);
    let len = string_len(&text) as f64;
    let relative = args
        .first()
        .copied()
        .map(|value| to_integer_or_infinity(host, value))
        .unwrap_or(0.0);
    let index = if relative.is_sign_negative() {
        len + relative
    } else {
        relative
    };
    if !index.is_finite() || index < 0.0 || index >= len {
        return make_undefined();
    }
    char_at(&text, index as usize)
        .map(|ch| host.intern_string(&ch))
        .unwrap_or_else(make_undefined)
}

fn string_char_at<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let text = string_value(host, this_value);
    let index = relative_index(host, args.first().copied(), string_len(&text), 0);
    char_at(&text, index)
        .map(|ch| host.intern_string(&ch))
        .unwrap_or_else(|| host.intern_string(""))
}

fn string_char_code_at<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    let text = string_value(host, this_value);
    let index = relative_index(host, args.first().copied(), string_len(&text), 0);
    char_at(&text, index)
        .and_then(|ch| ch.chars().next())
        .map(|ch| make_number(ch as u32 as f64))
        .unwrap_or_else(|| make_number(f64::NAN))
}

fn string_code_point_at<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    let text = string_value(host, this_value);
    let index = relative_index(host, args.first().copied(), string_len(&text), 0);
    char_at(&text, index)
        .and_then(|ch| ch.chars().next())
        .map(|ch| make_number(ch as u32 as f64))
        .unwrap_or_else(make_undefined)
}

fn string_concat<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let mut out = string_value(host, this_value);
    for &arg in args {
        out.push_str(&host.display_string(arg));
    }
    host.intern_string(&out)
}

fn string_ends_with<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    let text = string_value(host, this_value);
    let search = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_else(|| "undefined".to_owned());
    let len = string_len(&text);
    let end = relative_index(host, args.get(1).copied(), len, len);
    let search_len = string_len(&search);
    make_bool(search_len <= end && slice_chars(&text, end - search_len, end) == search)
}

fn string_includes<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let text = string_value(host, this_value);
    let search = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_else(|| "undefined".to_owned());
    let start = relative_index(host, args.get(1).copied(), string_len(&text), 0);
    make_bool(string_index_of(&text, &search, start).is_some())
}

fn string_index_of_method<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    let text = string_value(host, this_value);
    let search = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_else(|| "undefined".to_owned());
    let start = relative_index(host, args.get(1).copied(), string_len(&text), 0);
    make_number(string_index_of(&text, &search, start).map_or(-1.0, |index| index as f64))
}

fn string_last_index_of_method<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    let text = string_value(host, this_value);
    let search = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_else(|| "undefined".to_owned());
    let len = string_len(&text);
    let start = args
        .get(1)
        .copied()
        .map(|value| to_integer_or_infinity(host, value))
        .map(|value| {
            if value.is_nan() || value.is_infinite() {
                len
            } else {
                value.clamp(0.0, len as f64) as usize
            }
        })
        .unwrap_or(len);
    make_number(string_last_index_of(&text, &search, start).map_or(-1.0, |index| index as f64))
}

fn string_pad_end<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    string_pad(host, this_value, args, false)
}

fn string_pad_start<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    string_pad(host, this_value, args, true)
}

fn string_repeat<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let text = string_value(host, this_value);
    let count = args
        .first()
        .copied()
        .map(|value| to_integer_or_infinity(host, value))
        .unwrap_or(0.0);
    if count.is_sign_negative() {
        return host.intern_string("RangeError: repeat count must be non-negative");
    }
    if count.is_infinite() {
        return host.intern_string("RangeError: repeat count must be finite");
    }
    host.intern_string(&text.repeat(count as usize))
}

fn apply_replacement_template(template: &str, text: &str, matched: &str, index: usize) -> String {
    let prefix = slice_chars(text, 0, index);
    let suffix = slice_chars(text, index + string_len(matched), string_len(text));
    let mut out = String::new();
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }

        match chars.next() {
            Some('$') => out.push('$'),
            Some('&') => out.push_str(matched),
            Some('`') => out.push_str(&prefix),
            Some('\'') => out.push_str(&suffix),
            Some(other) => {
                out.push('$');
                out.push(other);
            }
            None => out.push('$'),
        }
    }

    out
}

fn regexp_exec_match<H: BuiltinHost>(
    host: &mut H,
    search_value: JSValue,
    text: &str,
) -> Option<Option<(usize, String)>> {
    if !host.is_object(search_value) {
        return None;
    }

    let exec = host.get_property(search_value, "exec");
    if !host.is_callable(exec) {
        return None;
    }

    host.set_property(search_value, "__qjs_regexp_last_index", make_number(0.0));
    let input = host.intern_string(text);
    let result = host.call_value(exec, search_value, &[input]);
    if result.is_null() || result.is_undefined() {
        return Some(None);
    }

    let matched = host.display_string(host.get_property_value(result, make_number(0.0)));
    let index = to_f64(host.number_value(host.get_property(result, "index")))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.trunc() as usize)
        .unwrap_or(0);

    Some(Some((index, matched)))
}

fn first_match<H: BuiltinHost>(
    host: &mut H,
    search_value: JSValue,
    text: &str,
) -> Option<(usize, String)> {
    if let Some(found) = regexp_exec_match(host, search_value, text) {
        return found;
    }

    let search = host.display_string(search_value);
    string_index_of(text, &search, 0).map(|index| (index, search))
}

fn string_replace<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let text = string_value(host, this_value);
    let search_value = args.first().copied().unwrap_or_else(make_undefined);
    let replace_value = args.get(1).copied().unwrap_or_else(make_undefined);

    let Some((index, matched)) = first_match(host, search_value, &text) else {
        return host.intern_string(&text);
    };

    let replacement = if host.is_callable(replace_value) {
        let matched_value = host.intern_string(&matched);
        let text_value = host.intern_string(&text);
        let value = host.call_value(
            replace_value,
            make_undefined(),
            &[matched_value, make_number(index as f64), text_value],
        );
        host.display_string(value)
    } else {
        apply_replacement_template(&host.display_string(replace_value), &text, &matched, index)
    };

    let prefix = slice_chars(&text, 0, index);
    let suffix = slice_chars(&text, index + string_len(&matched), string_len(&text));
    host.intern_string(&format!("{prefix}{replacement}{suffix}"))
}

fn string_replace_all<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    let text = string_value(host, this_value);
    let search = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_else(|| "undefined".to_owned());
    let replace_value = args.get(1).copied().unwrap_or_else(make_undefined);

    if search.is_empty() {
        return host.intern_string(&text);
    }

    let mut result = String::new();
    let mut cursor = 0;

    while let Some(index) = string_index_of(&text, &search, cursor) {
        result.push_str(&slice_chars(&text, cursor, index));
        let replacement = if host.is_callable(replace_value) {
            let matched_value = host.intern_string(&search);
            let text_value = host.intern_string(&text);
            let value = host.call_value(
                replace_value,
                make_undefined(),
                &[matched_value, make_number(index as f64), text_value],
            );
            host.display_string(value)
        } else {
            apply_replacement_template(&host.display_string(replace_value), &text, &search, index)
        };
        result.push_str(&replacement);
        cursor = index + string_len(&search);
    }

    result.push_str(&slice_chars(&text, cursor, string_len(&text)));
    host.intern_string(&result)
}

fn string_search<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let text = string_value(host, this_value);
    let search_value = args.first().copied().unwrap_or_else(make_undefined);
    make_number(first_match(host, search_value, &text).map_or(-1.0, |(index, _)| index as f64))
}

fn string_slice<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let text = string_value(host, this_value);
    let len = string_len(&text);
    let start = relative_index(host, args.first().copied(), len, 0);
    let end = relative_index(host, args.get(1).copied(), len, len);
    if start >= end {
        host.intern_string("")
    } else {
        host.intern_string(&slice_chars(&text, start, end))
    }
}

fn string_split<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let text = string_value(host, this_value);
    let limit = non_negative_limit(host, args.get(1).copied(), u32::MAX as usize);
    if limit == 0 {
        return host.create_array();
    }

    let Some(separator_value) = args.first().copied() else {
        let value = host.intern_string(&text);
        return create_array_from_values(host, [value]);
    };
    if separator_value.is_undefined() {
        let value = host.intern_string(&text);
        return create_array_from_values(host, [value]);
    }

    let separator = host.display_string(separator_value);
    if separator.is_empty() {
        let values = text
            .chars()
            .take(limit)
            .map(|ch| host.intern_string(&ch.to_string()))
            .collect::<Vec<_>>();
        return create_array_from_values(host, values);
    }

    let mut parts = Vec::new();
    let mut start = 0;
    while parts.len() < limit {
        let Some(index) = string_index_of(&text, &separator, start) else {
            break;
        };
        parts.push(host.intern_string(&slice_chars(&text, start, index)));
        start = index + string_len(&separator);
    }

    if parts.len() < limit {
        parts.push(host.intern_string(&slice_chars(&text, start, string_len(&text))));
    }

    create_array_from_values(host, parts)
}

fn string_starts_with<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    let text = string_value(host, this_value);
    let search = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_else(|| "undefined".to_owned());
    let len = string_len(&text);
    let start = relative_index(host, args.get(1).copied(), len, 0);
    make_bool(string_index_of(&text, &search, start).is_some_and(|index| index == start))
}

fn string_substring<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    let text = string_value(host, this_value);
    let len = string_len(&text);
    let start = relative_index(host, args.first().copied(), len, 0);
    let end = relative_index(host, args.get(1).copied(), len, len);
    host.intern_string(&slice_chars(&text, start.min(end), start.max(end)))
}

fn string_to_lower_case<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    let text = string_value(host, this_value).to_lowercase();
    host.intern_string(&text)
}

fn string_to_upper_case<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    let text = string_value(host, this_value).to_uppercase();
    host.intern_string(&text)
}

fn string_trim<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    let text = string_value(host, this_value);
    host.intern_string(text.trim())
}

fn string_trim_end<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    let text = string_value(host, this_value);
    host.intern_string(text.trim_end())
}

fn string_trim_start<H: BuiltinHost>(host: &mut H, this_value: JSValue) -> JSValue {
    let text = string_value(host, this_value);
    host.intern_string(text.trim_start())
}

fn string_pad<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
    at_start: bool,
) -> JSValue {
    let text = string_value(host, this_value);
    let len = string_len(&text);
    let target_len = non_negative_limit(host, args.first().copied(), len);
    if target_len <= len {
        return host.intern_string(&text);
    }

    let filler = args
        .get(1)
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_else(|| " ".to_owned());
    if filler.is_empty() {
        return host.intern_string(&text);
    }

    let fill_len = target_len - len;
    let filler_chars = filler.chars().collect::<Vec<_>>();
    let mut padding = String::new();
    for index in 0..fill_len {
        padding.push(filler_chars[index % filler_chars.len()]);
    }

    if at_start {
        host.intern_string(&format!("{padding}{text}"))
    } else {
        host.intern_string(&format!("{text}{padding}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_methods_are_registered() {
        assert_eq!(STRING_METHODS.len(), 28);
        assert!(
            STRING_METHODS
                .iter()
                .any(|method| method.property_name == "fromCharCode")
        );
        assert!(
            STRING_METHODS
                .iter()
                .any(|method| method.property_name == "repeat")
        );
        assert!(
            STRING_METHODS
                .iter()
                .any(|method| method.property_name == "replaceAll")
        );
        assert!(
            STRING_METHODS
                .iter()
                .any(|method| method.property_name == "trim")
        );
    }

    #[test]
    fn helper_searches_use_character_offsets() {
        assert_eq!(string_index_of("banana", "na", 0), Some(2));
        assert_eq!(string_last_index_of("banana", "na", 6), Some(4));
        assert_eq!(slice_chars("🙂hello", 0, 1), "🙂");
    }
}
