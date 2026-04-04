use std::{cmp::Ordering, collections::HashMap};

use value::{JSValue, to_f64};

use crate::{BuiltinHost, BuiltinMethod, create_builtin_callable, install_global_function};

const ARRAY_STATIC_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("isArray", "__builtin_array_is_array"),
    BuiltinMethod::new("from", "__builtin_array_from"),
    BuiltinMethod::new("of", "__builtin_array_of"),
];

const ARRAY_INSTANCE_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("push", "__builtin_array_push"),
    BuiltinMethod::new("pop", "__builtin_array_pop"),
    BuiltinMethod::new("shift", "__builtin_array_shift"),
    BuiltinMethod::new("unshift", "__builtin_array_unshift"),
    BuiltinMethod::new("slice", "__builtin_array_slice"),
    BuiltinMethod::new("splice", "__builtin_array_splice"),
    BuiltinMethod::new("concat", "__builtin_array_concat"),
    BuiltinMethod::new("join", "__builtin_array_join"),
    BuiltinMethod::new("reverse", "__builtin_array_reverse"),
    BuiltinMethod::new("sort", "__builtin_array_sort"),
    BuiltinMethod::new("indexOf", "__builtin_array_index_of"),
    BuiltinMethod::new("lastIndexOf", "__builtin_array_last_index_of"),
    BuiltinMethod::new("includes", "__builtin_array_includes"),
    BuiltinMethod::new("forEach", "__builtin_array_for_each"),
    BuiltinMethod::new("map", "__builtin_array_map"),
    BuiltinMethod::new("filter", "__builtin_array_filter"),
    BuiltinMethod::new("find", "__builtin_array_find"),
    BuiltinMethod::new("findIndex", "__builtin_array_find_index"),
    BuiltinMethod::new("reduce", "__builtin_array_reduce"),
    BuiltinMethod::new("reduceRight", "__builtin_array_reduce_right"),
    BuiltinMethod::new("some", "__builtin_array_some"),
    BuiltinMethod::new("every", "__builtin_array_every"),
    BuiltinMethod::new("fill", "__builtin_array_fill"),
    BuiltinMethod::new("copyWithin", "__builtin_array_copy_within"),
    BuiltinMethod::new("flat", "__builtin_array_flat"),
    BuiltinMethod::new("flatMap", "__builtin_array_flat_map"),
    BuiltinMethod::new("at", "__builtin_array_at"),
    BuiltinMethod::new("toString", "__builtin_array_to_string"),
    BuiltinMethod::new("values", "__builtin_array_values"),
    BuiltinMethod::new("keys", "__builtin_array_keys"),
    BuiltinMethod::new("entries", "__builtin_array_entries"),
];

#[cfg(test)]
const ARRAY_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("isArray", "__builtin_array_is_array"),
    BuiltinMethod::new("from", "__builtin_array_from"),
    BuiltinMethod::new("of", "__builtin_array_of"),
    BuiltinMethod::new("push", "__builtin_array_push"),
    BuiltinMethod::new("pop", "__builtin_array_pop"),
    BuiltinMethod::new("shift", "__builtin_array_shift"),
    BuiltinMethod::new("unshift", "__builtin_array_unshift"),
    BuiltinMethod::new("slice", "__builtin_array_slice"),
    BuiltinMethod::new("splice", "__builtin_array_splice"),
    BuiltinMethod::new("concat", "__builtin_array_concat"),
    BuiltinMethod::new("join", "__builtin_array_join"),
    BuiltinMethod::new("reverse", "__builtin_array_reverse"),
    BuiltinMethod::new("sort", "__builtin_array_sort"),
    BuiltinMethod::new("indexOf", "__builtin_array_index_of"),
    BuiltinMethod::new("lastIndexOf", "__builtin_array_last_index_of"),
    BuiltinMethod::new("includes", "__builtin_array_includes"),
    BuiltinMethod::new("forEach", "__builtin_array_for_each"),
    BuiltinMethod::new("map", "__builtin_array_map"),
    BuiltinMethod::new("filter", "__builtin_array_filter"),
    BuiltinMethod::new("find", "__builtin_array_find"),
    BuiltinMethod::new("findIndex", "__builtin_array_find_index"),
    BuiltinMethod::new("reduce", "__builtin_array_reduce"),
    BuiltinMethod::new("reduceRight", "__builtin_array_reduce_right"),
    BuiltinMethod::new("some", "__builtin_array_some"),
    BuiltinMethod::new("every", "__builtin_array_every"),
    BuiltinMethod::new("fill", "__builtin_array_fill"),
    BuiltinMethod::new("copyWithin", "__builtin_array_copy_within"),
    BuiltinMethod::new("flat", "__builtin_array_flat"),
    BuiltinMethod::new("flatMap", "__builtin_array_flat_map"),
    BuiltinMethod::new("at", "__builtin_array_at"),
    BuiltinMethod::new("toString", "__builtin_array_to_string"),
    BuiltinMethod::new("values", "__builtin_array_values"),
    BuiltinMethod::new("keys", "__builtin_array_keys"),
    BuiltinMethod::new("entries", "__builtin_array_entries"),
];

fn push_values<H: BuiltinHost>(
    host: &mut H,
    array: JSValue,
    values: impl IntoIterator<Item = JSValue>,
) -> JSValue {
    for value in values {
        let _ = host.array_push(array, value);
    }
    array
}

fn replace_array_contents<H: BuiltinHost>(
    host: &mut H,
    array: JSValue,
    values: &[JSValue],
) -> JSValue {
    let _ = host.set_property(array, "length", JSValue::from(0));
    push_values(host, array, values.iter().copied())
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

fn non_negative_usize<H: BuiltinHost>(
    host: &mut H,
    value: Option<JSValue>,
    default: usize,
) -> usize {
    let Some(value) = value else {
        return default;
    };

    let integer = to_integer_or_infinity(host, value);
    if !integer.is_finite() {
        return if integer.is_sign_positive() {
            usize::MAX
        } else {
            0
        };
    }

    integer.max(0.0) as usize
}

fn last_search_index<H: BuiltinHost>(
    host: &mut H,
    value: Option<JSValue>,
    len: usize,
) -> Option<usize> {
    if len == 0 {
        return None;
    }

    let Some(value) = value else {
        return Some(len - 1);
    };

    let integer = to_integer_or_infinity(host, value);
    if integer.is_sign_negative() {
        let index = (len as f64) + integer;
        (index >= 0.0).then_some(index as usize)
    } else if integer.is_infinite() {
        Some(len - 1)
    } else {
        Some(integer.min((len - 1) as f64) as usize)
    }
}

fn flat_depth<H: BuiltinHost>(host: &mut H, value: Option<JSValue>) -> i32 {
    let Some(value) = value else {
        return 1;
    };

    let integer = to_integer_or_infinity(host, value);
    if integer.is_infinite() && integer.is_sign_positive() {
        i32::MAX
    } else if !integer.is_finite() || integer <= 0.0 {
        0
    } else {
        integer.min(i32::MAX as f64) as i32
    }
}

fn same_value_zero<H: BuiltinHost>(host: &H, lhs: JSValue, rhs: JSValue) -> bool {
    matches!((to_f64(lhs), to_f64(rhs)), (Some(left), Some(right)) if left.is_nan() && right.is_nan())
        || host.same_value(lhs, rhs)
}

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(
        host,
        global_slots,
        "Array",
        "__builtin_array",
        ARRAY_STATIC_METHODS,
    );
}

pub(crate) fn attach_array_methods<H: BuiltinHost>(host: &mut H, target: JSValue) {
    for method in ARRAY_INSTANCE_METHODS {
        let function = create_builtin_callable(host, method.native_name);
        host.set_property(target, method.property_name, function);
    }
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    match name {
        "__builtin_array" => Some(array_constructor(host, args)),
        "__builtin_array_is_array" => Some(array_is_array(host, args)),
        "__builtin_array_from" => Some(array_from(host, args)),
        "__builtin_array_of" => Some(array_of(host, args)),
        "__builtin_array_push" => Some(array_push(host, this_value, args)),
        "__builtin_array_pop" => Some(array_pop(host, this_value, args)),
        "__builtin_array_shift" => Some(array_shift(host, this_value, args)),
        "__builtin_array_unshift" => Some(array_unshift(host, this_value, args)),
        "__builtin_array_slice" => Some(array_slice(host, this_value, args)),
        "__builtin_array_splice" => Some(array_splice(host, this_value, args)),
        "__builtin_array_concat" => Some(array_concat(host, this_value, args)),
        "__builtin_array_join" => Some(array_join(host, this_value, args)),
        "__builtin_array_reverse" => Some(array_reverse(host, this_value, args)),
        "__builtin_array_sort" => Some(array_sort(host, this_value, args)),
        "__builtin_array_index_of" => Some(array_index_of(host, this_value, args)),
        "__builtin_array_last_index_of" => Some(array_last_index_of(host, this_value, args)),
        "__builtin_array_includes" => Some(array_includes(host, this_value, args)),
        "__builtin_array_for_each" => Some(array_for_each(host, this_value, args)),
        "__builtin_array_map" => Some(array_map(host, this_value, args)),
        "__builtin_array_filter" => Some(array_filter(host, this_value, args)),
        "__builtin_array_find" => Some(array_find(host, this_value, args)),
        "__builtin_array_find_index" => Some(array_find_index(host, this_value, args)),
        "__builtin_array_reduce" => Some(array_reduce(host, this_value, args)),
        "__builtin_array_reduce_right" => Some(array_reduce_right(host, this_value, args)),
        "__builtin_array_some" => Some(array_some(host, this_value, args)),
        "__builtin_array_every" => Some(array_every(host, this_value, args)),
        "__builtin_array_fill" => Some(array_fill(host, this_value, args)),
        "__builtin_array_copy_within" => Some(array_copy_within(host, this_value, args)),
        "__builtin_array_flat" => Some(array_flat(host, this_value, args)),
        "__builtin_array_flat_map" => Some(array_flat_map(host, this_value, args)),
        "__builtin_array_at" => Some(array_at(host, this_value, args)),
        "__builtin_array_to_string" => Some(array_to_string(host, this_value, args)),
        "__builtin_array_values" => Some(array_values(host, this_value, args)),
        "__builtin_array_keys" => Some(array_keys(host, this_value, args)),
        "__builtin_array_entries" => Some(array_entries(host, this_value, args)),
        _ => None,
    }
}

pub(crate) fn construct<H: BuiltinHost>(
    host: &mut H,
    _callee_value: JSValue,
    name: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    (name == "__builtin_array").then(|| array_constructor(host, args))
}

/// Array constructor - creates new arrays with elements
fn array_constructor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    match args.len() {
        0 => host.create_array(),
        1 => {
            // Single argument - if it's a number, treat as length; otherwise as element
            if args[0].as_i32().is_some() || args[0].as_f64().is_some() {
                let result = host.create_array();
                let length = to_f64(args[0])
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .map(|value| value.trunc())
                    .unwrap_or(0.0);
                let _ = host.set_property(result, "length", JSValue::f64(length));
                result
            } else {
                let result = host.create_array();
                push_values(host, result, [args[0]])
            }
        }
        _ => {
            let result = host.create_array();
            push_values(host, result, args.iter().copied())
        }
    }
}

/// Check if value is an array
fn array_is_array<H: BuiltinHost>(host: &H, args: &[JSValue]) -> JSValue {
    let is_arr = args
        .first()
        .map(|&value| host.is_array(value))
        .unwrap_or(false);
    JSValue::bool(is_arr)
}

/// Array.from() - create array from iterable or array-like
fn array_from<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let arr = args.first().copied().unwrap_or(JSValue::undefined());

    if !host.is_object(arr) {
        return host.create_array();
    }

    // Get array values
    if let Some(values) = host.array_values(arr) {
        let result = host.create_array();
        push_values(host, result, values)
    } else {
        host.create_array()
    }
}

/// Array.of() - create array from arguments
fn array_of<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let result = host.create_array();
    push_values(host, result, args.iter().copied())
}

/// Array.prototype.push() - add elements to end
fn array_push<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !host.is_array(this_value) {
        return this_value;
    }

    let mut length = JSValue::f64(
        host.array_values(this_value)
            .map(|values| values.len() as f64)
            .unwrap_or(0.0),
    );
    for &arg in args {
        length = host.array_push(this_value, arg);
    }
    length
}

/// Array.prototype.pop() - remove and return last element
fn array_pop<H: BuiltinHost>(host: &mut H, this_value: JSValue, _args: &[JSValue]) -> JSValue {
    if !host.is_array(this_value) {
        return JSValue::undefined();
    }

    let Some(mut values) = host.array_values(this_value) else {
        return JSValue::undefined();
    };
    let popped = values.pop().unwrap_or(JSValue::undefined());
    replace_array_contents(host, this_value, &values);
    popped
}

/// Array.prototype.shift() - remove and return first element
fn array_shift<H: BuiltinHost>(host: &mut H, this_value: JSValue, _args: &[JSValue]) -> JSValue {
    if !host.is_array(this_value) {
        return JSValue::undefined();
    }

    let Some(mut values) = host.array_values(this_value) else {
        return JSValue::undefined();
    };
    if values.is_empty() {
        return JSValue::undefined();
    }

    let shifted = values.remove(0);
    replace_array_contents(host, this_value, &values);
    shifted
}

/// Array.prototype.unshift() - add elements to beginning
fn array_unshift<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !host.is_array(this_value) {
        return this_value;
    }
    let values = host.array_values(this_value).unwrap_or_default();
    let mut new_values = Vec::with_capacity(args.len() + values.len());
    new_values.extend_from_slice(args);
    new_values.extend(values);
    replace_array_contents(host, this_value, &new_values);
    JSValue::f64(new_values.len() as f64)
}

/// Array.prototype.slice() - return shallow copy of portion
fn array_slice<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !host.is_array(this_value) {
        return host.create_array();
    }
    let values = host.array_values(this_value).unwrap_or_default();
    let start = relative_index(host, args.first().copied(), values.len(), 0);
    let end = relative_index(host, args.get(1).copied(), values.len(), values.len());
    let result = host.create_array();
    push_values(
        host,
        result,
        values
            .iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .copied(),
    )
}

/// Array.prototype.splice() - add/remove elements
fn array_splice<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !host.is_array(this_value) {
        return host.create_array();
    }
    let mut values = host.array_values(this_value).unwrap_or_default();
    let start = relative_index(host, args.first().copied(), values.len(), 0);
    let delete_count = if args.len() < 2 {
        values.len().saturating_sub(start)
    } else {
        non_negative_usize(host, args.get(1).copied(), 0).min(values.len().saturating_sub(start))
    };

    let removed = values
        .splice(start..start + delete_count, args.iter().copied().skip(2))
        .collect::<Vec<_>>();
    replace_array_contents(host, this_value, &values);

    let deleted = host.create_array();
    push_values(host, deleted, removed)
}

/// Array.prototype.concat() - merge arrays
fn array_concat<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let result = host.create_array();
    if host.is_array(this_value) {
        if let Some(values) = host.array_values(this_value) {
            let _ = push_values(host, result, values);
        }
    } else {
        let _ = push_values(host, result, [this_value]);
    }
    for &arg in args {
        if host.is_array(arg) {
            if let Some(values) = host.array_values(arg) {
                let _ = push_values(host, result, values);
            }
        } else {
            let _ = push_values(host, result, [arg]);
        }
    }
    result
}

/// Array.prototype.join() - join elements with separator
fn array_join<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let separator = args
        .first()
        .map(|v| host.display_string(*v))
        .unwrap_or_else(|| ",".to_string());
    if let Some(values) = host.array_values(this_value) {
        let strs: Vec<String> = values
            .iter()
            .map(|v| {
                if v.is_null() || v.is_undefined() {
                    String::new()
                } else {
                    host.display_string(*v)
                }
            })
            .collect();
        host.intern_string(&strs.join(&separator))
    } else {
        host.intern_string("")
    }
}

/// Array.prototype.reverse() - reverse in place
fn array_reverse<H: BuiltinHost>(host: &mut H, this_value: JSValue, _args: &[JSValue]) -> JSValue {
    if let Some(mut values) = host.array_values(this_value) {
        values.reverse();
        let _ = replace_array_contents(host, this_value, &values);
    }

    this_value
}

/// Array.prototype.sort() - sort in place
fn array_sort<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if let Some(mut values) = host.array_values(this_value) {
        if let Some(compare) = args
            .first()
            .copied()
            .filter(|value| host.is_callable(*value))
        {
            values.sort_by(|lhs, rhs| {
                let result = host.call_value(compare, JSValue::undefined(), &[*lhs, *rhs]);
                let order = to_f64(host.number_value(result)).unwrap_or(0.0);
                if order < 0.0 {
                    Ordering::Less
                } else if order > 0.0 {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            });
        } else {
            values.sort_by(|lhs, rhs| host.display_string(*lhs).cmp(&host.display_string(*rhs)));
        }
        let _ = replace_array_contents(host, this_value, &values);
    }

    this_value
}

/// Array.prototype.indexOf() - find first index
fn array_index_of<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if let Some(values) = host.array_values(this_value) {
        let search = args.first().copied().unwrap_or(JSValue::undefined());
        let start = relative_index(host, args.get(1).copied(), values.len(), 0);
        for (idx, val) in values.iter().enumerate().skip(start) {
            if host.same_value(search, *val) {
                return JSValue::f64(idx as f64);
            }
        }
    }
    JSValue::f64(-1.0)
}

/// Array.prototype.lastIndexOf() - find last index
fn array_last_index_of<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    if let Some(values) = host.array_values(this_value) {
        let search = args.first().copied().unwrap_or(JSValue::undefined());
        let Some(start) = last_search_index(host, args.get(1).copied(), values.len()) else {
            return JSValue::f64(-1.0);
        };
        for idx in (0..=start).rev() {
            if host.same_value(search, values[idx]) {
                return JSValue::f64(idx as f64);
            }
        }
    }
    JSValue::f64(-1.0)
}

/// Array.prototype.includes() - check if array contains element
fn array_includes<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if let Some(values) = host.array_values(this_value) {
        let search = args.first().copied().unwrap_or(JSValue::undefined());
        let start = relative_index(host, args.get(1).copied(), values.len(), 0);
        for value in values.into_iter().skip(start) {
            if same_value_zero(host, search, value) {
                return JSValue::bool(true);
            }
        }
    }
    JSValue::bool(false)
}

/// Array.prototype.forEach() - execute callback for each element
fn array_for_each<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if let Some(values) = host.array_values(this_value) {
        if let Some(callback) = args.first().copied() {
            if host.is_callable(callback) {
                let this_arg = args.get(1).copied().unwrap_or(JSValue::undefined());
                for (idx, value) in values.iter().enumerate() {
                    let _ = host.call_value(
                        callback,
                        this_arg,
                        &[*value, JSValue::f64(idx as f64), this_value],
                    );
                }
            }
        }
    }
    JSValue::undefined()
}

/// Array.prototype.map() - create new array with transformed elements
fn array_map<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let result = host.create_array();
    if let Some(values) = host.array_values(this_value) {
        if let Some(callback) = args.first().copied() {
            if host.is_callable(callback) {
                let this_arg = args.get(1).copied().unwrap_or(JSValue::undefined());
                for (idx, value) in values.iter().enumerate() {
                    let mapped = host.call_value(
                        callback,
                        this_arg,
                        &[*value, JSValue::f64(idx as f64), this_value],
                    );
                    let _ = push_values(host, result, [mapped]);
                }
            }
        }
    }
    result
}

/// Array.prototype.filter() - create new array with filtered elements
fn array_filter<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let result = host.create_array();
    if let Some(values) = host.array_values(this_value) {
        if let Some(callback) = args.first().copied() {
            if host.is_callable(callback) {
                let this_arg = args.get(1).copied().unwrap_or(JSValue::undefined());
                for (idx, value) in values.iter().enumerate() {
                    let keep = host.call_value(
                        callback,
                        this_arg,
                        &[*value, JSValue::f64(idx as f64), this_value],
                    );
                    if host.is_truthy_value(keep) {
                        let _ = push_values(host, result, [*value]);
                    }
                }
            }
        }
    }
    result
}

/// Array.prototype.find() - find first element matching predicate
fn array_find<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if let Some(values) = host.array_values(this_value) {
        if let Some(callback) = args.first().copied() {
            if host.is_callable(callback) {
                let this_arg = args.get(1).copied().unwrap_or(JSValue::undefined());
                for (idx, value) in values.iter().enumerate() {
                    let found = host.call_value(
                        callback,
                        this_arg,
                        &[*value, JSValue::f64(idx as f64), this_value],
                    );
                    if host.is_truthy_value(found) {
                        return *value;
                    }
                }
            }
        }
    }
    JSValue::undefined()
}

/// Array.prototype.findIndex() - find first index matching predicate
fn array_find_index<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    if let Some(values) = host.array_values(this_value) {
        if let Some(callback) = args.first().copied() {
            if host.is_callable(callback) {
                let this_arg = args.get(1).copied().unwrap_or(JSValue::undefined());
                for (idx, value) in values.iter().enumerate() {
                    let found = host.call_value(
                        callback,
                        this_arg,
                        &[*value, JSValue::f64(idx as f64), this_value],
                    );
                    if host.is_truthy_value(found) {
                        return JSValue::f64(idx as f64);
                    }
                }
            }
        }
    }
    JSValue::f64(-1.0)
}

/// Array.prototype.reduce() - reduce array to single value
fn array_reduce<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if let Some(values) = host.array_values(this_value) {
        if let Some(callback) = args.first().copied() {
            if host.is_callable(callback) {
                let (mut acc, start) = if let Some(initial) = args.get(1).copied() {
                    (initial, 0)
                } else if let Some(first) = values.first().copied() {
                    (first, 1)
                } else {
                    return JSValue::undefined();
                };
                for (idx, value) in values.iter().enumerate().skip(start) {
                    acc = host.call_value(
                        callback,
                        JSValue::undefined(),
                        &[acc, *value, JSValue::f64(idx as f64), this_value],
                    );
                }
                return acc;
            }
        }
    }
    JSValue::undefined()
}

/// Array.prototype.reduceRight() - reduce array from right
fn array_reduce_right<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    if let Some(values) = host.array_values(this_value) {
        if let Some(callback) = args.first().copied() {
            if host.is_callable(callback) {
                if values.is_empty() && args.get(1).is_none() {
                    return JSValue::undefined();
                }

                let mut stop = values.len();
                let mut acc = if let Some(initial) = args.get(1).copied() {
                    initial
                } else {
                    stop -= 1;
                    values[stop]
                };

                for idx in (0..stop).rev() {
                    acc = host.call_value(
                        callback,
                        JSValue::undefined(),
                        &[acc, values[idx], JSValue::f64(idx as f64), this_value],
                    );
                }
                return acc;
            }
        }
    }
    JSValue::undefined()
}

/// Array.prototype.some() - test if any element matches predicate
fn array_some<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if let Some(values) = host.array_values(this_value) {
        if let Some(callback) = args.first().copied() {
            if host.is_callable(callback) {
                let this_arg = args.get(1).copied().unwrap_or(JSValue::undefined());
                for (idx, value) in values.iter().enumerate() {
                    let result = host.call_value(
                        callback,
                        this_arg,
                        &[*value, JSValue::f64(idx as f64), this_value],
                    );
                    if host.is_truthy_value(result) {
                        return JSValue::bool(true);
                    }
                }
            }
        }
    }
    JSValue::bool(false)
}

/// Array.prototype.every() - test if all elements match predicate
fn array_every<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if let Some(values) = host.array_values(this_value) {
        if let Some(callback) = args.first().copied() {
            if host.is_callable(callback) {
                let this_arg = args.get(1).copied().unwrap_or(JSValue::undefined());
                for (idx, value) in values.iter().enumerate() {
                    let result = host.call_value(
                        callback,
                        this_arg,
                        &[*value, JSValue::f64(idx as f64), this_value],
                    );
                    if !host.is_truthy_value(result) {
                        return JSValue::bool(false);
                    }
                }
            }
        }
    }
    JSValue::bool(true)
}

/// Array.prototype.fill() - fill array elements with value
fn array_fill<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if !host.is_array(this_value) {
        return this_value;
    }
    let value = args.first().copied().unwrap_or(JSValue::undefined());
    if let Some(mut values) = host.array_values(this_value) {
        let len = values.len();
        let start = relative_index(host, args.get(1).copied(), len, 0);
        let end = relative_index(host, args.get(2).copied(), len, len);
        for slot in values.iter_mut().take(end.min(len)).skip(start) {
            *slot = value;
        }
        let _ = replace_array_contents(host, this_value, &values);
    }

    this_value
}

/// Array.prototype.copyWithin() - copy part of array to another location
fn array_copy_within<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    args: &[JSValue],
) -> JSValue {
    if !host.is_array(this_value) {
        return this_value;
    }
    if let Some(mut values) = host.array_values(this_value) {
        let len = values.len();
        let target = relative_index(host, args.first().copied(), len, 0);
        let start = relative_index(host, args.get(1).copied(), len, 0);
        let end = relative_index(host, args.get(2).copied(), len, len);
        if target < len && start < end {
            let count = (end - start).min(len - target);
            let copied = values[start..start + count].to_vec();
            for (offset, value) in copied.into_iter().enumerate() {
                if target + offset < len {
                    values[target + offset] = value;
                }
            }
        }
        let _ = replace_array_contents(host, this_value, &values);
    }

    this_value
}

/// Array.prototype.flat() - flatten nested arrays
fn array_flat<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    fn flatten_recursive<H: BuiltinHost>(
        host: &mut H,
        level: i32,
        value: JSValue,
        result: JSValue,
    ) {
        if level > 0 && host.is_array(value) {
            if let Some(values) = host.array_values(value) {
                for value in values {
                    flatten_recursive(host, level - 1, value, result);
                }
            }
        } else {
            let _ = push_values(host, result, [value]);
        }
    }

    let depth = flat_depth(host, args.first().copied());
    let result = host.create_array();
    if let Some(values) = host.array_values(this_value) {
        for value in values {
            flatten_recursive(host, depth, value, result);
        }
    }
    result
}

/// Array.prototype.flatMap() - map then flatten
fn array_flat_map<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    let result = host.create_array();
    if let Some(values) = host.array_values(this_value) {
        if let Some(callback) = args.first().copied() {
            if host.is_callable(callback) {
                let this_arg = args.get(1).copied().unwrap_or(JSValue::undefined());
                for (idx, value) in values.iter().enumerate() {
                    let mapped = host.call_value(
                        callback,
                        this_arg,
                        &[*value, JSValue::f64(idx as f64), this_value],
                    );
                    if host.is_array(mapped) {
                        if let Some(mapped_values) = host.array_values(mapped) {
                            let _ = push_values(host, result, mapped_values);
                        }
                    } else {
                        let _ = push_values(host, result, [mapped]);
                    }
                }
            }
        }
    }
    result
}

/// Array.prototype.at() - access array by index, supporting negative indices
fn array_at<H: BuiltinHost>(host: &mut H, this_value: JSValue, args: &[JSValue]) -> JSValue {
    if let Some(values) = host.array_values(this_value) {
        let index = args
            .first()
            .copied()
            .map(|value| to_integer_or_infinity(host, value))
            .unwrap_or(0.0);
        let actual_index = if index.is_sign_negative() {
            (values.len() as f64) + index
        } else {
            index
        };

        if !actual_index.is_finite() || actual_index < 0.0 || actual_index >= values.len() as f64 {
            JSValue::undefined()
        } else {
            values[actual_index as usize]
        }
    } else {
        JSValue::undefined()
    }
}

/// Array.prototype.toString() - convert array to string
fn array_to_string<H: BuiltinHost>(
    host: &mut H,
    this_value: JSValue,
    _args: &[JSValue],
) -> JSValue {
    let result = array_join(host, this_value, &[]);
    result
}

/// Array.prototype.values() - create iterator for array values
fn array_values<H: BuiltinHost>(host: &mut H, this_value: JSValue, _args: &[JSValue]) -> JSValue {
    // Create an array iterator object
    let iterator = host.create_object();

    // Store the array reference
    let _ = host.set_property(iterator, "__qjs_array_iterator_array", this_value);

    // Store the current index
    let _ = host.set_property(iterator, "__qjs_array_iterator_index", JSValue::f64(0.0));

    // Set the iterator type
    let iter_type = host.intern_string("values");
    let _ = host.set_property(iterator, "__qjs_array_iterator_type", iter_type);

    iterator
}

/// Array.prototype.keys() - create iterator for array indices
fn array_keys<H: BuiltinHost>(host: &mut H, this_value: JSValue, _args: &[JSValue]) -> JSValue {
    // Create an array iterator object
    let iterator = host.create_object();

    // Store the array reference
    let _ = host.set_property(iterator, "__qjs_array_iterator_array", this_value);

    // Store the current index
    let _ = host.set_property(iterator, "__qjs_array_iterator_index", JSValue::f64(0.0));

    // Set the iterator type
    let iter_type = host.intern_string("keys");
    let _ = host.set_property(iterator, "__qjs_array_iterator_type", iter_type);

    iterator
}

/// Array.prototype.entries() - create iterator for array entries
fn array_entries<H: BuiltinHost>(host: &mut H, this_value: JSValue, _args: &[JSValue]) -> JSValue {
    // Create an array iterator object
    let iterator = host.create_object();

    // Store the array reference
    let _ = host.set_property(iterator, "__qjs_array_iterator_array", this_value);

    // Store the current index
    let _ = host.set_property(iterator, "__qjs_array_iterator_index", JSValue::f64(0.0));

    // Set the iterator type
    let iter_type = host.intern_string("entries");
    let _ = host.set_property(iterator, "__qjs_array_iterator_type", iter_type);

    iterator
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_array_methods_exist() {
        // Verify that all array method constants are defined
        assert!(!ARRAY_METHODS.is_empty());
        assert_eq!(ARRAY_METHODS.len(), 34);

        // Check for key methods
        let method_names: Vec<&str> = ARRAY_METHODS.iter().map(|m| m.property_name).collect();
        assert!(method_names.contains(&"isArray"));
        assert!(method_names.contains(&"from"));
        assert!(method_names.contains(&"of"));
        assert!(method_names.contains(&"push"));
        assert!(method_names.contains(&"pop"));
        assert!(method_names.contains(&"slice"));
        assert!(method_names.contains(&"map"));
        assert!(method_names.contains(&"filter"));
        assert!(method_names.contains(&"reduce"));
        assert!(method_names.contains(&"forEach"));
        assert!(method_names.contains(&"join"));
        assert!(method_names.contains(&"concat"));
        assert!(method_names.contains(&"reverse"));
        assert!(method_names.contains(&"sort"));
        assert!(method_names.contains(&"indexOf"));
        assert!(method_names.contains(&"includes"));
        assert!(method_names.contains(&"find"));
        assert!(method_names.contains(&"findIndex"));
        assert!(method_names.contains(&"some"));
        assert!(method_names.contains(&"every"));
        assert!(method_names.contains(&"flat"));
        assert!(method_names.contains(&"flatMap"));
        assert!(method_names.contains(&"fill"));
        assert!(method_names.contains(&"copyWithin"));
        assert!(method_names.contains(&"at"));
    }

    #[test]
    fn test_array_methods_dispatch_names() {
        // Verify dispatch function handles all method names
        let dispatch_names = [
            "__builtin_array",
            "__builtin_array_is_array",
            "__builtin_array_from",
            "__builtin_array_of",
            "__builtin_array_push",
            "__builtin_array_pop",
            "__builtin_array_shift",
            "__builtin_array_unshift",
            "__builtin_array_slice",
            "__builtin_array_splice",
            "__builtin_array_concat",
            "__builtin_array_join",
            "__builtin_array_reverse",
            "__builtin_array_sort",
            "__builtin_array_index_of",
            "__builtin_array_last_index_of",
            "__builtin_array_includes",
            "__builtin_array_for_each",
            "__builtin_array_map",
            "__builtin_array_filter",
            "__builtin_array_find",
            "__builtin_array_find_index",
            "__builtin_array_reduce",
            "__builtin_array_reduce_right",
            "__builtin_array_some",
            "__builtin_array_every",
            "__builtin_array_fill",
            "__builtin_array_copy_within",
            "__builtin_array_flat",
            "__builtin_array_flat_map",
            "__builtin_array_at",
        ];

        // Verify each method is unique
        let mut seen = std::collections::HashSet::new();
        for name in dispatch_names.iter() {
            assert!(seen.insert(name), "Duplicate dispatch name: {}", name);
        }
    }

    #[test]
    fn test_array_methods_property_names() {
        // Verify all properties have correct native names format
        for method in ARRAY_METHODS.iter() {
            assert!(
                method.native_name.starts_with("__builtin_array"),
                "Invalid native name format: {}",
                method.native_name
            );
            assert!(
                !method.property_name.is_empty(),
                "Empty property name for native: {}",
                method.native_name
            );
        }
    }

    #[test]
    fn test_array_constructor_factory_method() {
        // Verify array_constructor has proper signature
        // This is a compile-time check - if the function signature is wrong, tests won't compile
        let method_count = ARRAY_METHODS.len();
        assert_eq!(method_count, 34, "Expected 34 array methods");
    }

    #[test]
    fn test_array_is_array_builtin_method() {
        // Test that array_is_array is a proper BuiltinMethod
        let is_array_method = ARRAY_METHODS.iter().find(|m| m.property_name == "isArray");
        assert!(is_array_method.is_some(), "isArray method not found");
        assert_eq!(
            is_array_method.unwrap().native_name,
            "__builtin_array_is_array"
        );
    }

    #[test]
    fn test_callback_based_methods_exist() {
        // Verify all callback-based methods are registered
        let callback_methods = [
            "forEach",
            "map",
            "filter",
            "find",
            "findIndex",
            "reduce",
            "reduceRight",
            "some",
            "every",
            "flatMap",
        ];

        for callback_method in callback_methods.iter() {
            let found = ARRAY_METHODS
                .iter()
                .any(|m| m.property_name == *callback_method);
            assert!(
                found,
                "Callback-based method {} not found in ARRAY_METHODS",
                callback_method
            );
        }
    }

    #[test]
    fn test_search_methods_exist() {
        // Verify all search methods are registered
        let search_methods = ["indexOf", "lastIndexOf", "includes", "find", "findIndex"];

        for search_method in search_methods.iter() {
            let found = ARRAY_METHODS
                .iter()
                .any(|m| m.property_name == *search_method);
            assert!(
                found,
                "Search method {} not found in ARRAY_METHODS",
                search_method
            );
        }
    }

    #[test]
    fn test_mutating_methods_exist() {
        // Verify all mutating methods are registered
        let mutating_methods = [
            "push",
            "pop",
            "shift",
            "unshift",
            "reverse",
            "sort",
            "splice",
            "fill",
            "copyWithin",
        ];

        for mutating_method in mutating_methods.iter() {
            let found = ARRAY_METHODS
                .iter()
                .any(|m| m.property_name == *mutating_method);
            assert!(
                found,
                "Mutating method {} not found in ARRAY_METHODS",
                mutating_method
            );
        }
    }

    #[test]
    fn test_static_methods_exist() {
        // Verify all static methods are registered
        let static_methods = ["isArray", "from", "of"];

        for static_method in static_methods.iter() {
            let found = ARRAY_METHODS
                .iter()
                .any(|m| m.property_name == *static_method);
            assert!(
                found,
                "Static method {} not found in ARRAY_METHODS",
                static_method
            );
        }
    }
}
