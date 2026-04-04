use std::collections::HashMap;

use value::{JSValue, make_number, make_undefined, to_f64};

use crate::{
    BuiltinHost, BuiltinMethod, attach_callable_methods as attach_callable_methods_entry,
    create_array_from_values, install_global_function, install_methods,
};

const CALLABLE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("call", "__builtin_function_call"),
    BuiltinMethod::new("apply", "__builtin_function_apply"),
    BuiltinMethod::new("bind", "__builtin_function_bind"),
    BuiltinMethod::new("toString", "__builtin_function_to_string"),
];

const BOUND_TARGET_PROP: &str = "__qjs_bound_target";
const BOUND_THIS_PROP: &str = "__qjs_bound_this";
const BOUND_ARGS_PROP: &str = "__qjs_bound_args";
const DYNAMIC_PARAMS_PROP: &str = "__qjs_function_params";
const DYNAMIC_BODY_PROP: &str = "__qjs_function_body";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(host, global_slots, "Function", "__builtin_function", &[]);
}

pub(crate) fn attach_callable_methods<H: BuiltinHost>(host: &mut H, target: JSValue) {
    install_methods(host, target, CALLABLE_METHODS);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_function" => Some(function_constructor(host, args)),
        "__builtin_function_empty" => Some(make_undefined()),
        "__builtin_dynamic_function" => Some(dynamic_function_call(host, callee_value, args)),
        "__builtin_function_call" => Some(function_call(host, this_value, args)),
        "__builtin_function_apply" => Some(function_apply(host, this_value, args)),
        "__builtin_function_bind" => Some(function_bind(host, this_value, args)),
        "__builtin_function_to_string" => Some(function_to_string(host, this_value)),
        "__builtin_bound_function" => Some(bound_function_call(host, callee_value, args)),
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_function" => Some(function_constructor(host, args)),
        "__builtin_dynamic_function" => Some(dynamic_function_construct(host, callee_value, args)),
        "__builtin_bound_function" => Some(bound_function_construct(host, callee_value, args)),
        _ => None,
    }
}

pub(crate) fn function_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let (body, param_names) = match args.split_last() {
        Some((body, params)) => (
            host.display_string(*body),
            params
                .iter()
                .map(|&value| host.display_string(value))
                .collect::<Vec<_>>(),
        ),
        None => (String::new(), Vec::new()),
    };

    let function = host.builtin_function("__builtin_dynamic_function");
    attach_callable_methods_entry(host, function);
    let param_values = param_names
        .into_iter()
        .map(|name| host.intern_string(&name))
        .collect::<Vec<_>>();
    let params = create_array_from_values(host, param_values);
    let body_value = host.intern_string(&body);
    host.set_property(function, DYNAMIC_PARAMS_PROP, params);
    host.set_property(function, DYNAMIC_BODY_PROP, body_value);
    function
}

fn function_call<H: BuiltinHost>(host: &mut H, target: JSValue, args: &[JSValue]) -> JSValue {
    if !host.is_callable(target) {
        return make_undefined();
    }

    let this_arg = args.first().copied().unwrap_or_else(make_undefined);
    host.call_value(target, this_arg, &args[1..])
}

fn function_apply<H: BuiltinHost>(host: &mut H, target: JSValue, args: &[JSValue]) -> JSValue {
    if !host.is_callable(target) {
        return make_undefined();
    }

    let this_arg = args.first().copied().unwrap_or_else(make_undefined);
    let applied_args = args
        .get(1)
        .copied()
        .and_then(|value| host.array_values(value))
        .unwrap_or_default();
    host.call_value(target, this_arg, &applied_args)
}

fn function_bind<H: BuiltinHost>(host: &mut H, target: JSValue, args: &[JSValue]) -> JSValue {
    if !host.is_callable(target) {
        return make_undefined();
    }

    let bound = host.builtin_function("__builtin_bound_function");
    attach_callable_methods_entry(host, bound);
    host.set_property(bound, BOUND_TARGET_PROP, target);
    host.set_property(
        bound,
        BOUND_THIS_PROP,
        args.first().copied().unwrap_or_else(make_undefined),
    );
    let bound_args = create_array_from_values(host, args.iter().copied().skip(1));
    host.set_property(bound, BOUND_ARGS_PROP, bound_args);
    bound
}

fn function_to_string<H: BuiltinHost>(host: &mut H, target: JSValue) -> JSValue {
    let rendered = host.display_string(target);
    host.intern_string(&rendered)
}

fn bound_function_call<H: BuiltinHost>(host: &mut H, bound: JSValue, args: &[JSValue]) -> JSValue {
    let target = host.get_property(bound, BOUND_TARGET_PROP);
    if !host.is_callable(target) {
        return make_undefined();
    }

    let this_arg = host.get_property(bound, BOUND_THIS_PROP);
    let mut combined = host
        .array_values(host.get_property(bound, BOUND_ARGS_PROP))
        .unwrap_or_default();
    combined.extend_from_slice(args);
    host.call_value(target, this_arg, &combined)
}

fn bound_function_construct<H: BuiltinHost>(
    host: &mut H,
    bound: JSValue,
    args: &[JSValue],
) -> JSValue {
    let target = host.get_property(bound, BOUND_TARGET_PROP);
    if !host.is_callable(target) {
        return host.create_object();
    }

    let mut combined = host
        .array_values(host.get_property(bound, BOUND_ARGS_PROP))
        .unwrap_or_default();
    combined.extend_from_slice(args);
    host.construct_value(target, &combined)
}

fn dynamic_function_call<H: BuiltinHost>(
    host: &mut H,
    function: JSValue,
    args: &[JSValue],
) -> JSValue {
    let params = host
        .array_values(host.get_property(function, DYNAMIC_PARAMS_PROP))
        .unwrap_or_default()
        .into_iter()
        .map(|value| {
            host.string_text(value)
                .unwrap_or_default()
                .trim()
                .to_owned()
        })
        .collect::<Vec<_>>();
    let body = host
        .string_text(host.get_property(function, DYNAMIC_BODY_PROP))
        .unwrap_or_default()
        .to_owned();
    evaluate_function_body(host, &params, &body, args)
}

fn dynamic_function_construct<H: BuiltinHost>(
    host: &mut H,
    function: JSValue,
    args: &[JSValue],
) -> JSValue {
    let result = dynamic_function_call(host, function, args);
    if host.is_object(result) {
        result
    } else {
        host.create_object()
    }
}

fn evaluate_function_body<H: BuiltinHost>(
    host: &mut H,
    params: &[String],
    body: &str,
    args: &[JSValue],
) -> JSValue {
    let expr = body.trim();
    if expr.is_empty() {
        return make_undefined();
    }

    let expr = expr
        .strip_prefix("return")
        .map(str::trim_start)
        .unwrap_or(expr)
        .strip_suffix(';')
        .map(str::trim_end)
        .unwrap_or(expr)
        .trim();
    if expr.is_empty() {
        return make_undefined();
    }

    let bindings = params
        .iter()
        .enumerate()
        .map(|(index, name)| {
            (
                name.clone(),
                args.get(index).copied().unwrap_or_else(make_undefined),
            )
        })
        .collect::<HashMap<_, _>>();
    Parser::new(host, expr, &bindings)
        .parse_expression()
        .unwrap_or_else(make_undefined)
}

struct Parser<'a, H: BuiltinHost> {
    host: &'a mut H,
    input: &'a str,
    index: usize,
    bindings: &'a HashMap<String, JSValue>,
}

impl<'a, H: BuiltinHost> Parser<'a, H> {
    fn new(host: &'a mut H, input: &'a str, bindings: &'a HashMap<String, JSValue>) -> Self {
        Self {
            host,
            input,
            index: 0,
            bindings,
        }
    }

    fn parse_expression(&mut self) -> Option<JSValue> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            if self.consume('+') {
                let rhs = self.parse_term()?;
                value = self.add(value, rhs);
            } else if self.consume('-') {
                let rhs = self.parse_term()?;
                value = self.numeric_bin(value, rhs, |lhs, rhs| lhs - rhs);
            } else {
                break;
            }
        }
        Some(value)
    }

    fn parse_term(&mut self) -> Option<JSValue> {
        let mut value = self.parse_factor()?;
        loop {
            self.skip_ws();
            if self.consume('*') {
                let rhs = self.parse_factor()?;
                value = self.numeric_bin(value, rhs, |lhs, rhs| lhs * rhs);
            } else if self.consume('/') {
                let rhs = self.parse_factor()?;
                value = self.numeric_bin(value, rhs, |lhs, rhs| lhs / rhs);
            } else if self.consume('%') {
                let rhs = self.parse_factor()?;
                value = self.numeric_bin(value, rhs, |lhs, rhs| lhs % rhs);
            } else {
                break;
            }
        }
        Some(value)
    }

    fn parse_factor(&mut self) -> Option<JSValue> {
        self.skip_ws();
        if self.consume('(') {
            let value = self.parse_expression()?;
            self.skip_ws();
            let _ = self.consume(')');
            return Some(value);
        }
        if self.consume('-') {
            let value = self.parse_factor()?;
            return Some(self.numeric_bin(make_number(0.0), value, |lhs, rhs| lhs - rhs));
        }
        if let Some(value) = self.parse_string_literal() {
            return Some(value);
        }
        if let Some(value) = self.parse_number_literal() {
            return Some(value);
        }
        if let Some(identifier) = self.parse_identifier() {
            return Some(
                self.bindings
                    .get(&identifier)
                    .copied()
                    .unwrap_or_else(make_undefined),
            );
        }
        None
    }

    fn parse_string_literal(&mut self) -> Option<JSValue> {
        let quote = self.peek()?;
        if quote != '\'' && quote != '"' {
            return None;
        }
        self.index += quote.len_utf8();
        let start = self.index;
        while let Some(ch) = self.peek() {
            if ch == quote {
                let text = self.input[start..self.index].to_owned();
                self.index += quote.len_utf8();
                return Some(self.host.intern_string(&text));
            }
            self.index += ch.len_utf8();
        }
        None
    }

    fn parse_number_literal(&mut self) -> Option<JSValue> {
        let start = self.index;
        let mut seen_digit = false;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() || ch == '.' {
                seen_digit = true;
                self.index += ch.len_utf8();
            } else {
                break;
            }
        }
        if !seen_digit {
            self.index = start;
            return None;
        }
        self.input[start..self.index]
            .parse::<f64>()
            .ok()
            .map(make_number)
    }

    fn parse_identifier(&mut self) -> Option<String> {
        let start = self.index;
        let first = self.peek()?;
        if !(first == '_' || first == '$' || first.is_ascii_alphabetic()) {
            return None;
        }
        self.index += first.len_utf8();
        while let Some(ch) = self.peek() {
            if ch == '_' || ch == '$' || ch.is_ascii_alphanumeric() {
                self.index += ch.len_utf8();
            } else {
                break;
            }
        }
        Some(self.input[start..self.index].to_owned())
    }

    fn add(&mut self, lhs: JSValue, rhs: JSValue) -> JSValue {
        if self.host.string_text(lhs).is_some() || self.host.string_text(rhs).is_some() {
            let rendered = format!(
                "{}{}",
                self.host.display_string(lhs),
                self.host.display_string(rhs)
            );
            self.host.intern_string(&rendered)
        } else {
            self.numeric_bin(lhs, rhs, |left, right| left + right)
        }
    }

    fn numeric_bin(&mut self, lhs: JSValue, rhs: JSValue, op: fn(f64, f64) -> f64) -> JSValue {
        let lhs = to_f64(self.host.number_value(lhs)).unwrap_or(f64::NAN);
        let rhs = to_f64(self.host.number_value(rhs)).unwrap_or(f64::NAN);
        make_number(op(lhs, rhs))
    }

    fn skip_ws(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.index += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.index += expected.len_utf8();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.index..].chars().next()
    }
}
