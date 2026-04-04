use std::collections::HashMap;

use value::{JSValue, make_undefined};

use crate::{BuiltinHost, BuiltinMethod, install_global_object};

const CONSOLE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("log", "__builtin_console_log"),
    BuiltinMethod::new("error", "__builtin_console_error"),
    BuiltinMethod::new("warn", "__builtin_console_warn"),
    BuiltinMethod::new("info", "__builtin_console_info"),
    BuiltinMethod::new("debug", "__builtin_console_debug"),
    BuiltinMethod::new("trace", "__builtin_console_trace"),
    BuiltinMethod::new("table", "__builtin_console_table"),
    BuiltinMethod::new("time", "__builtin_console_time"),
    BuiltinMethod::new("timeEnd", "__builtin_console_time_end"),
    BuiltinMethod::new("group", "__builtin_console_group"),
    BuiltinMethod::new("groupEnd", "__builtin_console_group_end"),
    BuiltinMethod::new("clear", "__builtin_console_clear"),
    BuiltinMethod::new("count", "__builtin_console_count"),
    BuiltinMethod::new("assert", "__builtin_console_assert"),
    BuiltinMethod::new("dir", "__builtin_console_dir"),
    BuiltinMethod::new("dirxml", "__builtin_console_dirxml"),
    BuiltinMethod::new("timeLog", "__builtin_console_time_log"),
];

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_object(host, global_slots, "console", CONSOLE_METHODS);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_console_log" => Some(log_like(host, args, false)),
        "__builtin_console_error" => Some(log_like(host, args, true)),
        "__builtin_console_warn" => Some(log_like(host, args, true)),
        "__builtin_console_info" => Some(log_like(host, args, false)),
        "__builtin_console_debug" => Some(log_like(host, args, false)),
        "__builtin_console_trace" => Some(console_trace(host, args)),
        "__builtin_console_table" => Some(log_like(host, args, false)),
        "__builtin_console_time" => Some(console_time(host, args)),
        "__builtin_console_time_end" => Some(console_time_end(host, args)),
        "__builtin_console_group" => Some(console_group(host, args)),
        "__builtin_console_group_end" => Some(console_group_end(host)),
        "__builtin_console_clear" => Some(console_clear(host)),
        "__builtin_console_count" => Some(console_count(host, args)),
        "__builtin_console_assert" => Some(console_assert(host, args)),
        "__builtin_console_dir" => Some(log_like(host, args, false)),
        "__builtin_console_dirxml" => Some(log_like(host, args, false)),
        "__builtin_console_time_log" => Some(console_time_log(host, args)),
        _ => None,
    }
}

fn log_like<H: BuiltinHost>(host: &mut H, args: &[JSValue], is_error: bool) -> JSValue {
    let text = host.console_render_args(args);
    host.console_write_line(text, is_error)
}

fn console_trace<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let message = host.console_render_args(args);
    let line = if message.is_empty() {
        "Trace".to_owned()
    } else {
        format!("Trace: {message}")
    };

    host.console_write_line(line, true)
}

fn console_time<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let label = host.console_label_from_args(args);
    host.console_time_start(label);
    make_undefined()
}

fn console_time_end<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let label = host.console_label_from_args(args);
    match host.console_time_end(&label) {
        Some(start) => {
            host.console_write_line(host.console_elapsed_message(&label, start, None), false)
        }
        None => host.console_write_line(format!("Timer '{label}' does not exist"), true),
    }
}

fn console_group<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    if !args.is_empty() {
        let line = host.console_render_args(args);
        host.console_write_line(line, false);
    }

    host.console_group_start();
    make_undefined()
}

fn console_group_end<H: BuiltinHost>(host: &mut H) -> JSValue {
    host.console_group_end();
    make_undefined()
}

fn console_clear<H: BuiltinHost>(host: &mut H) -> JSValue {
    host.console_clear();
    make_undefined()
}

fn console_count<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let label = host.console_label_from_args(args);
    let next = host.console_count_increment(&label);
    host.console_write_line(format!("{label}: {next}"), false)
}

fn console_assert<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let condition = args.first().copied().unwrap_or_else(make_undefined);
    if host.is_truthy_value(condition) {
        return make_undefined();
    }

    let message = if args.len() <= 1 {
        "Assertion failed".to_owned()
    } else {
        format!("Assertion failed: {}", host.console_render_args(&args[1..]))
    };

    host.console_write_line(message, true)
}

fn console_time_log<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let label = host.console_label_from_args(args);
    let extra = if args.len() <= 1 {
        None
    } else {
        Some(host.console_render_args(&args[1..]))
    };

    match host.console_time_get(&label) {
        Some(start) => host.console_write_line(
            host.console_elapsed_message(&label, start, extra.as_deref()),
            false,
        ),
        None => host.console_write_line(format!("Timer '{label}' does not exist"), true),
    }
}
