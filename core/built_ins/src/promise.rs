use std::collections::HashMap;

use value::{JSValue, make_number, make_undefined, to_f64};

use crate::{BuiltinHost, BuiltinMethod, install_global_function, install_methods};

const PROMISE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("resolve", "__builtin_promise_resolve_static"),
    BuiltinMethod::new("reject", "__builtin_promise_reject_static"),
    BuiltinMethod::new("all", "__builtin_promise_all"),
];

const INSTANCE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("then", "__builtin_promise_then"),
    BuiltinMethod::new("catch", "__builtin_promise_catch"),
    BuiltinMethod::new("finally", "__builtin_promise_finally"),
];

const KIND_PROP: &str = "__qjs_builtin_kind";
const STATE_PROP: &str = "__qjs_promise_state";
const RESULT_PROP: &str = "__qjs_promise_result";
const REACTIONS_PROP: &str = "__qjs_promise_reactions";

const PROMISE_PROP: &str = "__qjs_promise_ref";
const REACTION_KIND_PROP: &str = "__qjs_reaction_kind";
const REACTION_FULFILLED_PROP: &str = "__qjs_reaction_fulfilled";
const REACTION_REJECTED_PROP: &str = "__qjs_reaction_rejected";
const REACTION_NEXT_PROMISE_PROP: &str = "__qjs_reaction_next_promise";
const REACTION_CALLBACK_PROP: &str = "__qjs_reaction_callback";

const ALL_OUT_PROP: &str = "__qjs_promise_all_out";
const ALL_RESULTS_PROP: &str = "__qjs_promise_all_results";
const ALL_INDEX_PROP: &str = "__qjs_promise_all_index";
const ALL_COUNTER_PROP: &str = "__qjs_promise_all_counter";
const COUNTER_VALUE_PROP: &str = "__qjs_promise_counter_value";

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(
        host,
        global_slots,
        "Promise",
        "__builtin_promise",
        PROMISE_METHODS,
    );
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_promise" => Some(promise_constructor(host, args)),
        "__builtin_promise_resolve_static" => Some(promise_resolve_static(host, args)),
        "__builtin_promise_reject_static" => Some(promise_reject_static(host, args)),
        "__builtin_promise_all" => Some(promise_all(host, args)),
        "__builtin_promise_then" => Some(promise_then_method(host, this_value, args)),
        "__builtin_promise_catch" => Some(promise_catch_method(host, this_value, args)),
        "__builtin_promise_finally" => Some(promise_finally_method(host, this_value, args)),
        "__builtin_promise_resolve_function" => {
            Some(promise_resolve_function(host, callee_value, args))
        }
        "__builtin_promise_reject_function" => {
            Some(promise_reject_function(host, callee_value, args))
        }
        "__builtin_promise_all_resolve_element" => {
            Some(promise_all_resolve_element(host, callee_value, args))
        }
        "__builtin_promise_all_reject_element" => {
            Some(promise_all_reject_element(host, callee_value, args))
        }
        "__builtin_await_unwrap" => {
            // Internal function to extract value from a Promise or pass-through non-Promise values
            Some(await_unwrap(host, args))
        }
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    (name == "__builtin_promise").then(|| promise_constructor(host, args))
}

fn promise_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let promise = create_promise(host);
    let executor = args.first().copied().unwrap_or_else(make_undefined);
    if host.is_callable(executor) {
        let resolve = host.builtin_function("__builtin_promise_resolve_function");
        let reject = host.builtin_function("__builtin_promise_reject_function");
        host.set_property(resolve, PROMISE_PROP, promise);
        host.set_property(reject, PROMISE_PROP, promise);
        let _ = host.call_value(executor, make_undefined(), &[resolve, reject]);
    }
    promise
}

fn promise_resolve_static<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let value = args.first().copied().unwrap_or_else(make_undefined);
    if is_promise(host, value) {
        return value;
    }

    let promise = create_promise(host);
    settle_promise(host, promise, "fulfilled", value);
    promise
}

fn promise_reject_static<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let promise = create_promise(host);
    settle_promise(
        host,
        promise,
        "rejected",
        args.first().copied().unwrap_or_else(make_undefined),
    );
    promise
}

fn promise_all<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let values = args
        .first()
        .copied()
        .and_then(|value| host.array_values(value))
        .unwrap_or_default();
    let out = create_promise(host);
    let results = host.create_array();
    let counter = host.create_object();
    host.set_property(
        counter,
        COUNTER_VALUE_PROP,
        make_number(values.len() as f64),
    );

    if values.is_empty() {
        settle_promise(host, out, "fulfilled", results);
        return out;
    }

    for (index, value) in values.into_iter().enumerate() {
        if promise_state(host, out) == "rejected" {
            break;
        }

        if is_promise(host, value) {
            match promise_state(host, value).as_str() {
                "fulfilled" => {
                    host.set_index(results, index, promise_result(host, value));
                    decrement_all_counter(host, out, results, counter);
                }
                "rejected" => {
                    settle_promise(host, out, "rejected", promise_result(host, value));
                }
                _ => {
                    let resolve = host.builtin_function("__builtin_promise_all_resolve_element");
                    host.set_property(resolve, ALL_OUT_PROP, out);
                    host.set_property(resolve, ALL_RESULTS_PROP, results);
                    host.set_property(resolve, ALL_INDEX_PROP, make_number(index as f64));
                    host.set_property(resolve, ALL_COUNTER_PROP, counter);

                    let reject = host.builtin_function("__builtin_promise_all_reject_element");
                    host.set_property(reject, ALL_OUT_PROP, out);
                    let reaction = create_then_reaction(host, resolve, reject, make_undefined());

                    add_reaction(host, value, reaction);
                }
            }
        } else {
            host.set_index(results, index, value);
            decrement_all_counter(host, out, results, counter);
        }
    }

    out
}

fn promise_then_method<H: BuiltinHost>(
    host: &mut H,
    promise: JSValue,
    args: &[JSValue],
) -> JSValue {
    let on_fulfilled = args.first().copied().unwrap_or_else(make_undefined);
    let on_rejected = args.get(1).copied().unwrap_or_else(make_undefined);
    promise_then(host, promise, on_fulfilled, on_rejected)
}

fn promise_catch_method<H: BuiltinHost>(
    host: &mut H,
    promise: JSValue,
    args: &[JSValue],
) -> JSValue {
    let on_rejected = args.first().copied().unwrap_or_else(make_undefined);
    promise_then(host, promise, make_undefined(), on_rejected)
}

fn promise_finally_method<H: BuiltinHost>(
    host: &mut H,
    promise: JSValue,
    args: &[JSValue],
) -> JSValue {
    let next = create_promise(host);
    let reaction = create_finally_reaction(
        host,
        args.first().copied().unwrap_or_else(make_undefined),
        next,
    );
    if promise_state(host, promise) == "pending" {
        add_reaction(host, promise, reaction);
    } else {
        run_reaction(host, promise, reaction);
    }
    next
}

fn promise_resolve_function<H: BuiltinHost>(
    host: &mut H,
    callee: JSValue,
    args: &[JSValue],
) -> JSValue {
    let promise = host.get_property(callee, PROMISE_PROP);
    let value = args.first().copied().unwrap_or_else(make_undefined);
    settle_promise(host, promise, "fulfilled", value);
    promise
}

fn promise_reject_function<H: BuiltinHost>(
    host: &mut H,
    callee: JSValue,
    args: &[JSValue],
) -> JSValue {
    let promise = host.get_property(callee, PROMISE_PROP);
    let value = args.first().copied().unwrap_or_else(make_undefined);
    settle_promise(host, promise, "rejected", value);
    promise
}

fn promise_all_resolve_element<H: BuiltinHost>(
    host: &mut H,
    callee: JSValue,
    args: &[JSValue],
) -> JSValue {
    let out = host.get_property(callee, ALL_OUT_PROP);
    if promise_state(host, out) != "pending" {
        return out;
    }

    let results = host.get_property(callee, ALL_RESULTS_PROP);
    let index = to_f64(host.number_value(host.get_property(callee, ALL_INDEX_PROP)))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.trunc() as usize)
        .unwrap_or(0);
    host.set_index(
        results,
        index,
        args.first().copied().unwrap_or_else(make_undefined),
    );
    decrement_all_counter(
        host,
        out,
        results,
        host.get_property(callee, ALL_COUNTER_PROP),
    );
    out
}

fn promise_all_reject_element<H: BuiltinHost>(
    host: &mut H,
    callee: JSValue,
    args: &[JSValue],
) -> JSValue {
    let out = host.get_property(callee, ALL_OUT_PROP);
    if promise_state(host, out) == "pending" {
        settle_promise(
            host,
            out,
            "rejected",
            args.first().copied().unwrap_or_else(make_undefined),
        );
    }
    out
}

fn create_promise<H: BuiltinHost>(host: &mut H) -> JSValue {
    let promise = host.create_object();
    let kind = host.intern_string("Promise");
    let state = host.intern_string("pending");
    let reactions = host.create_array();
    host.set_property(promise, KIND_PROP, kind);
    host.set_property(promise, STATE_PROP, state);
    host.set_property(promise, RESULT_PROP, make_undefined());
    host.set_property(promise, REACTIONS_PROP, reactions);
    install_methods(host, promise, INSTANCE_METHODS);
    promise
}

fn is_promise<H: BuiltinHost>(host: &H, value: JSValue) -> bool {
    host.string_text(host.get_property(value, KIND_PROP)) == Some("Promise")
}

fn promise_state<H: BuiltinHost>(host: &H, promise: JSValue) -> String {
    host.string_text(host.get_property(promise, STATE_PROP))
        .unwrap_or("pending")
        .to_owned()
}

fn promise_result<H: BuiltinHost>(host: &H, promise: JSValue) -> JSValue {
    host.get_property(promise, RESULT_PROP)
}

fn settle_promise<H: BuiltinHost>(host: &mut H, promise: JSValue, state: &str, value: JSValue) {
    if !is_promise(host, promise) || promise_state(host, promise) != "pending" {
        return;
    }

    if state == "fulfilled" && is_promise(host, value) {
        if value == promise {
            let rejected = host.intern_string("rejected");
            host.set_property(promise, STATE_PROP, rejected);
            host.set_property(promise, RESULT_PROP, make_undefined());
            flush_reactions(host, promise);
            return;
        }

        match promise_state(host, value).as_str() {
            "pending" => {
                let reaction =
                    create_then_reaction(host, make_undefined(), make_undefined(), promise);
                add_reaction(host, value, reaction);
                return;
            }
            "fulfilled" => {
                let fulfilled = host.intern_string("fulfilled");
                host.set_property(promise, STATE_PROP, fulfilled);
                host.set_property(promise, RESULT_PROP, promise_result(host, value));
                flush_reactions(host, promise);
                return;
            }
            "rejected" => {
                let rejected = host.intern_string("rejected");
                host.set_property(promise, STATE_PROP, rejected);
                host.set_property(promise, RESULT_PROP, promise_result(host, value));
                flush_reactions(host, promise);
                return;
            }
            _ => {}
        }
    }

    let state_value = host.intern_string(state);
    host.set_property(promise, STATE_PROP, state_value);
    host.set_property(promise, RESULT_PROP, value);
    flush_reactions(host, promise);
}

fn promise_then<H: BuiltinHost>(
    host: &mut H,
    promise: JSValue,
    on_fulfilled: JSValue,
    on_rejected: JSValue,
) -> JSValue {
    let next = create_promise(host);
    let reaction = create_then_reaction(host, on_fulfilled, on_rejected, next);
    if promise_state(host, promise) == "pending" {
        add_reaction(host, promise, reaction);
    } else {
        run_reaction(host, promise, reaction);
    }
    next
}

fn add_reaction<H: BuiltinHost>(host: &mut H, promise: JSValue, reaction: JSValue) {
    let reactions = host.get_property(promise, REACTIONS_PROP);
    let _ = host.array_push(reactions, reaction);
}

fn flush_reactions<H: BuiltinHost>(host: &mut H, promise: JSValue) {
    let reactions = host
        .array_values(host.get_property(promise, REACTIONS_PROP))
        .unwrap_or_default();
    let next_reactions = host.create_array();
    host.set_property(promise, REACTIONS_PROP, next_reactions);
    for reaction in reactions {
        run_reaction(host, promise, reaction);
    }
}

fn run_reaction<H: BuiltinHost>(host: &mut H, promise: JSValue, reaction: JSValue) {
    let state = promise_state(host, promise);
    let value = promise_result(host, promise);
    let kind = host
        .string_text(host.get_property(reaction, REACTION_KIND_PROP))
        .unwrap_or("then");

    if kind == "finally" {
        let callback = host.get_property(reaction, REACTION_CALLBACK_PROP);
        if host.is_callable(callback) {
            let _ = host.call_value(callback, make_undefined(), &[]);
        }
        let next = host.get_property(reaction, REACTION_NEXT_PROMISE_PROP);
        if is_promise(host, next) {
            settle_promise(host, next, &state, value);
        }
        return;
    }

    let handler = if state == "fulfilled" {
        host.get_property(reaction, REACTION_FULFILLED_PROP)
    } else {
        host.get_property(reaction, REACTION_REJECTED_PROP)
    };
    let next = host.get_property(reaction, REACTION_NEXT_PROMISE_PROP);

    if host.is_callable(handler) {
        let outcome = host.call_value(handler, make_undefined(), &[value]);
        if is_promise(host, next) {
            settle_promise(host, next, "fulfilled", outcome);
        }
    } else if is_promise(host, next) {
        settle_promise(host, next, &state, value);
    }
}

fn create_then_reaction<H: BuiltinHost>(
    host: &mut H,
    on_fulfilled: JSValue,
    on_rejected: JSValue,
    next_promise: JSValue,
) -> JSValue {
    let reaction = host.create_object();
    let kind = host.intern_string("then");
    host.set_property(reaction, REACTION_KIND_PROP, kind);
    host.set_property(reaction, REACTION_FULFILLED_PROP, on_fulfilled);
    host.set_property(reaction, REACTION_REJECTED_PROP, on_rejected);
    host.set_property(reaction, REACTION_NEXT_PROMISE_PROP, next_promise);
    reaction
}

fn create_finally_reaction<H: BuiltinHost>(
    host: &mut H,
    callback: JSValue,
    next_promise: JSValue,
) -> JSValue {
    let reaction = host.create_object();
    let kind = host.intern_string("finally");
    host.set_property(reaction, REACTION_KIND_PROP, kind);
    host.set_property(reaction, REACTION_CALLBACK_PROP, callback);
    host.set_property(reaction, REACTION_NEXT_PROMISE_PROP, next_promise);
    reaction
}

fn decrement_all_counter<H: BuiltinHost>(
    host: &mut H,
    out: JSValue,
    results: JSValue,
    counter: JSValue,
) {
    let next = to_f64(host.number_value(host.get_property(counter, COUNTER_VALUE_PROP)))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value - 1.0)
        .unwrap_or(0.0);
    host.set_property(counter, COUNTER_VALUE_PROP, make_number(next.max(0.0)));
    if next <= 0.0 && promise_state(host, out) == "pending" {
        settle_promise(host, out, "fulfilled", results);
    }
}

/// Internal function for await support
/// Returns the resolved value from a Promise, or the value itself if not a Promise
fn await_unwrap<H: BuiltinHost>(host: &H, args: &[JSValue]) -> JSValue {
    let value = args.first().copied().unwrap_or_else(make_undefined);
    
    // Check if the value is a Promise
    if is_promise(host, value) {
        // Get the Promise's state and result
        let state = promise_state(host, value);
        match state.as_str() {
            "fulfilled" => promise_result(host, value),
            "rejected" => promise_result(host, value),
            "pending" | _ => {
                // For pending Promises, we can't truly await in the current architecture
                // Return the Promise itself (this limitation requires async/await transformation)
                value
            }
        }
    } else {
        // Non-Promise values are passed through
        value
    }
}
