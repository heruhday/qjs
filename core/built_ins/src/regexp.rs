use std::collections::HashMap;

use value::{JSValue, make_bool, make_null, make_number, make_undefined};

use crate::{
    BuiltinHost, BuiltinMethod, create_array_from_values, install_global_function, install_methods,
};

const INSTANCE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("test", "__builtin_regexp_test"),
    BuiltinMethod::new("exec", "__builtin_regexp_exec"),
    BuiltinMethod::new("toString", "__builtin_regexp_to_string"),
];

const KIND_PROP: &str = "__qjs_builtin_kind";
const SOURCE_PROP: &str = "__qjs_regexp_source";
const FLAGS_PROP: &str = "__qjs_regexp_flags";
const LAST_INDEX_PROP: &str = "__qjs_regexp_last_index";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(host, global_slots, "RegExp", "__builtin_regexp", &[]);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_regexp" => Some(regexp_constructor(host, args)),
        "__builtin_regexp_test" => Some(regexp_test(host, this_value, args)),
        "__builtin_regexp_exec" => Some(regexp_exec(host, this_value, args)),
        "__builtin_regexp_to_string" => Some(regexp_to_string(host, this_value)),
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    (name == "__builtin_regexp").then(|| regexp_constructor(host, args))
}

fn regexp_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    if args
        .first()
        .copied()
        .is_some_and(|value| is_regexp_instance(host, value))
        && args.get(1).is_none()
    {
        return args[0];
    }

    let source = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_default();
    let flags = args
        .get(1)
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_default();

    let regexp = host.create_object();
    let kind = host.intern_string("RegExp");
    let source_value = host.intern_string(&source);
    let flags_value = host.intern_string(&flags);
    host.set_property(regexp, KIND_PROP, kind);
    host.set_property(regexp, SOURCE_PROP, source_value);
    host.set_property(regexp, FLAGS_PROP, flags_value);
    host.set_property(regexp, LAST_INDEX_PROP, make_number(0.0));
    install_methods(host, regexp, INSTANCE_METHODS);
    regexp
}

fn regexp_test<H: BuiltinHost>(host: &mut H, regexp: JSValue, args: &[JSValue]) -> JSValue {
    make_bool(regexp_match(host, regexp, args).is_some())
}

fn regexp_exec<H: BuiltinHost>(host: &mut H, regexp: JSValue, args: &[JSValue]) -> JSValue {
    let Some((index, matched)) = regexp_match(host, regexp, args) else {
        return make_null();
    };

    let matched_value = host.intern_string(&matched);
    let result = create_array_from_values(host, [matched_value]);
    let input_value = args
        .first()
        .copied()
        .map(|value| {
            let rendered = host.display_string(value);
            host.intern_string(&rendered)
        })
        .unwrap_or_else(make_undefined);
    host.set_property(result, "index", make_number(index as f64));
    host.set_property(result, "input", input_value);
    result
}

fn regexp_to_string<H: BuiltinHost>(host: &mut H, regexp: JSValue) -> JSValue {
    let source = host
        .string_text(host.get_property(regexp, SOURCE_PROP))
        .unwrap_or_default()
        .to_owned();
    let flags = host
        .string_text(host.get_property(regexp, FLAGS_PROP))
        .unwrap_or_default()
        .to_owned();
    host.intern_string(&format!("/{source}/{flags}"))
}

fn is_regexp_instance<H: BuiltinHost>(host: &H, value: JSValue) -> bool {
    host.string_text(host.get_property(value, KIND_PROP)) == Some("RegExp")
}

fn regexp_match<H: BuiltinHost>(
    host: &mut H,
    regexp: JSValue,
    args: &[JSValue],
) -> Option<(usize, String)> {
    if !is_regexp_instance(host, regexp) {
        return None;
    }

    let input = args
        .first()
        .copied()
        .map(|value| host.display_string(value))
        .unwrap_or_default();
    let source = host
        .string_text(host.get_property(regexp, SOURCE_PROP))
        .unwrap_or_default()
        .to_owned();
    let flags = host
        .string_text(host.get_property(regexp, FLAGS_PROP))
        .unwrap_or_default()
        .to_owned();
    let global = flags.contains('g');
    let start = if global {
        value::to_f64(host.number_value(host.get_property(regexp, LAST_INDEX_PROP)))
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value.trunc() as usize)
            .unwrap_or(0)
    } else {
        0
    };

    let matched = find_match(&source, &input, start, flags.contains('i'));
    match matched {
        Some((index, value)) => {
            if global {
                host.set_property(
                    regexp,
                    LAST_INDEX_PROP,
                    make_number((index + value.chars().count()) as f64),
                );
            }
            Some((index, value))
        }
        None => {
            if global {
                host.set_property(regexp, LAST_INDEX_PROP, make_number(0.0));
            }
            None
        }
    }
}

fn find_match(
    pattern: &str,
    input: &str,
    start: usize,
    ignore_case: bool,
) -> Option<(usize, String)> {
    let mut anchored_start = false;
    let mut anchored_end = false;
    let mut raw = pattern;

    if let Some(stripped) = raw.strip_prefix('^') {
        anchored_start = true;
        raw = stripped;
    }
    if let Some(stripped) = raw.strip_suffix('$') {
        anchored_end = true;
        raw = stripped;
    }

    let Some(tokens) = parse_simple_tokens(raw) else {
        return literal_match(raw, input, start, ignore_case, anchored_start, anchored_end);
    };

    let chars = input.chars().collect::<Vec<_>>();
    if anchored_start && start != 0 {
        return None;
    }

    let positions = if anchored_start {
        vec![start]
    } else {
        (start..=chars.len()).collect::<Vec<_>>()
    };

    for index in positions {
        if let Some(end) = matches_simple(&tokens, &chars, index, ignore_case)
            && (!anchored_end || end == chars.len())
        {
            let matched = chars[index..end].iter().collect::<String>();
            return Some((index, matched));
        }
    }

    None
}

fn literal_match(
    pattern: &str,
    input: &str,
    start: usize,
    ignore_case: bool,
    anchored_start: bool,
    anchored_end: bool,
) -> Option<(usize, String)> {
    let input_chars = input.chars().collect::<Vec<_>>();
    let pattern_chars = pattern.chars().collect::<Vec<_>>();
    if anchored_start && start != 0 {
        return None;
    }

    let positions = if anchored_start {
        vec![start]
    } else {
        (start..=input_chars.len()).collect::<Vec<_>>()
    };

    for index in positions {
        let end = index + pattern_chars.len();
        if end > input_chars.len() {
            continue;
        }
        if anchored_end && end != input_chars.len() {
            continue;
        }
        if input_chars[index..end]
            .iter()
            .zip(pattern_chars.iter())
            .all(|(&left, &right)| chars_equal(left, right, ignore_case))
        {
            return Some((index, input_chars[index..end].iter().collect::<String>()));
        }
    }

    None
}

fn parse_simple_tokens(pattern: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '.' => tokens.push(Token::Any),
            '\\' => match chars.next() {
                Some(next) => tokens.push(Token::Literal(next)),
                None => tokens.push(Token::Literal('\\')),
            },
            '*' | '+' | '?' | '[' | ']' | '(' | ')' | '{' | '}' | '|' => return None,
            _ => tokens.push(Token::Literal(ch)),
        }
    }
    Some(tokens)
}

fn matches_simple(
    tokens: &[Token],
    chars: &[char],
    start: usize,
    ignore_case: bool,
) -> Option<usize> {
    let mut index = start;
    for token in tokens {
        let current = *chars.get(index)?;
        match token {
            Token::Any => {}
            Token::Literal(expected) if chars_equal(current, *expected, ignore_case) => {}
            Token::Literal(_) => return None,
        }
        index += 1;
    }
    Some(index)
}

fn chars_equal(left: char, right: char, ignore_case: bool) -> bool {
    if ignore_case {
        left.to_lowercase().to_string() == right.to_lowercase().to_string()
    } else {
        left == right
    }
}

enum Token {
    Any,
    Literal(char),
}
