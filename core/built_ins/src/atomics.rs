// Atomics implementation for QuickJS
// Provides atomic operations on SharedArrayBuffer-backed TypedArrays

use std::collections::HashMap;

use value::{JSValue, make_number, make_undefined, to_f64, to_i32};

use crate::{BuiltinHost, BuiltinMethod, install_global_object};

const ATOMICS_METHODS: &[BuiltinMethod] = &[
    BuiltinMethod::new("add", "__builtin_atomics_add"),
    BuiltinMethod::new("and", "__builtin_atomics_and"),
    BuiltinMethod::new("compareExchange", "__builtin_atomics_compare_exchange"),
    BuiltinMethod::new("exchange", "__builtin_atomics_exchange"),
    BuiltinMethod::new("isLockFree", "__builtin_atomics_is_lock_free"),
    BuiltinMethod::new("load", "__builtin_atomics_load"),
    BuiltinMethod::new("notify", "__builtin_atomics_notify"),
    BuiltinMethod::new("or", "__builtin_atomics_or"),
    BuiltinMethod::new("store", "__builtin_atomics_store"),
    BuiltinMethod::new("sub", "__builtin_atomics_sub"),
    BuiltinMethod::new("waitAsync", "__builtin_atomics_wait_async"),
    BuiltinMethod::new("wait", "__builtin_atomics_wait"),
    BuiltinMethod::new("xor", "__builtin_atomics_xor"),
];

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _atomics = install_global_object(host, global_slots, "Atomics", ATOMICS_METHODS);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    _this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    Some(match name {
        "__builtin_atomics_load" => atomics_load(host, args),
        "__builtin_atomics_store" => atomics_store(host, args),
        "__builtin_atomics_add" => atomics_add(host, args),
        "__builtin_atomics_sub" => atomics_sub(host, args),
        "__builtin_atomics_and" => atomics_and(host, args),
        "__builtin_atomics_or" => atomics_or(host, args),
        "__builtin_atomics_xor" => atomics_xor(host, args),
        "__builtin_atomics_exchange" => atomics_exchange(host, args),
        "__builtin_atomics_compare_exchange" => atomics_compare_exchange(host, args),
        "__builtin_atomics_wait" => atomics_wait(host, args),
        "__builtin_atomics_notify" => atomics_notify(host, args),
        "__builtin_atomics_wait_async" => atomics_wait_async(host, args),
        "__builtin_atomics_is_lock_free" => atomics_is_lock_free(host, args),
        _ => return None,
    })
}

/// Atomics.load(typedArray, index)
/// Atomically reads a value from the specified position in an array
fn atomics_load<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let typed_array = args.first().copied().unwrap_or(make_undefined());
    let index = args
        .get(1)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0) as usize;

    if !host.is_object(typed_array) {
        return make_undefined();
    }

    // Get the value at the specified index from the typed array
    host.get_index(typed_array, index)
}

/// Atomics.store(typedArray, index, value)
/// Atomically writes a value to the specified position in an array
fn atomics_store<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let typed_array = args.first().copied().unwrap_or(make_undefined());
    let index = args
        .get(1)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0) as usize;
    let value = args.get(2).copied().unwrap_or(make_undefined());

    if !host.is_object(typed_array) {
        return make_undefined();
    }

    let num_value = to_f64(host.number_value(value)).unwrap_or(0.0);
    host.set_index(typed_array, index, make_number(num_value));
    make_number(num_value)
}

/// Atomics.add(typedArray, index, value)
/// Atomically adds a value to the element at the specified position
fn atomics_add<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let typed_array = args.first().copied().unwrap_or(make_undefined());
    let index = args
        .get(1)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0) as usize;
    let add_value = args
        .get(2)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0);

    if !host.is_object(typed_array) {
        return make_undefined();
    }

    let old_value = host.get_index(typed_array, index);
    let old_num = to_i32(host.number_value(old_value)).unwrap_or(0);
    let new_value = old_num.wrapping_add(add_value);

    host.set_index(typed_array, index, make_number(new_value as f64));
    make_number(old_num as f64)
}

/// Atomics.sub(typedArray, index, value)
/// Atomically subtracts a value from the element at the specified position
fn atomics_sub<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let typed_array = args.first().copied().unwrap_or(make_undefined());
    let index = args
        .get(1)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0) as usize;
    let sub_value = args
        .get(2)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0);

    if !host.is_object(typed_array) {
        return make_undefined();
    }

    let old_value = host.get_index(typed_array, index);
    let old_num = to_i32(host.number_value(old_value)).unwrap_or(0);
    let new_value = old_num.wrapping_sub(sub_value);

    host.set_index(typed_array, index, make_number(new_value as f64));
    make_number(old_num as f64)
}

/// Atomics.and(typedArray, index, value)
/// Atomically performs a bitwise AND operation
fn atomics_and<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let typed_array = args.first().copied().unwrap_or(make_undefined());
    let index = args
        .get(1)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0) as usize;
    let and_value = args
        .get(2)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0);

    if !host.is_object(typed_array) {
        return make_undefined();
    }

    let old_value = host.get_index(typed_array, index);
    let old_num = to_i32(host.number_value(old_value)).unwrap_or(0);
    let new_value = old_num & and_value;

    host.set_index(typed_array, index, make_number(new_value as f64));
    make_number(old_num as f64)
}

/// Atomics.or(typedArray, index, value)
/// Atomically performs a bitwise OR operation
fn atomics_or<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let typed_array = args.first().copied().unwrap_or(make_undefined());
    let index = args
        .get(1)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0) as usize;
    let or_value = args
        .get(2)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0);

    if !host.is_object(typed_array) {
        return make_undefined();
    }

    let old_value = host.get_index(typed_array, index);
    let old_num = to_i32(host.number_value(old_value)).unwrap_or(0);
    let new_value = old_num | or_value;

    host.set_index(typed_array, index, make_number(new_value as f64));
    make_number(old_num as f64)
}

/// Atomics.xor(typedArray, index, value)
/// Atomically performs a bitwise XOR operation
fn atomics_xor<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let typed_array = args.first().copied().unwrap_or(make_undefined());
    let index = args
        .get(1)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0) as usize;
    let xor_value = args
        .get(2)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0);

    if !host.is_object(typed_array) {
        return make_undefined();
    }

    let old_value = host.get_index(typed_array, index);
    let old_num = to_i32(host.number_value(old_value)).unwrap_or(0);
    let new_value = old_num ^ xor_value;

    host.set_index(typed_array, index, make_number(new_value as f64));
    make_number(old_num as f64)
}

/// Atomics.exchange(typedArray, index, value)
/// Atomically exchanges a value at the specified position
fn atomics_exchange<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let typed_array = args.first().copied().unwrap_or(make_undefined());
    let index = args
        .get(1)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0) as usize;
    let new_value = args
        .get(2)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0);

    if !host.is_object(typed_array) {
        return make_undefined();
    }

    let old_value = host.get_index(typed_array, index);
    host.set_index(typed_array, index, make_number(new_value as f64));
    old_value
}

/// Atomics.compareExchange(typedArray, index, expectedValue, replacementValue)
/// Atomically compares and exchanges a value
fn atomics_compare_exchange<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let typed_array = args.first().copied().unwrap_or(make_undefined());
    let index = args
        .get(1)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0) as usize;
    let expected = args
        .get(2)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0);
    let replacement = args
        .get(3)
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0);

    if !host.is_object(typed_array) {
        return make_undefined();
    }

    let old_value = host.get_index(typed_array, index);
    let old_num = to_i32(host.number_value(old_value)).unwrap_or(0);

    if old_num == expected {
        host.set_index(typed_array, index, make_number(replacement as f64));
    }

    make_number(old_num as f64)
}

/// Atomics.wait(typedArray, index, value, timeout)
/// Synchronously waits until the typed array element is notified or times out
fn atomics_wait<H: BuiltinHost>(host: &mut H, _args: &[JSValue]) -> JSValue {
    // For now, return "not-equal" since we don't have worker support yet
    host.intern_string("not-equal")
}

/// Atomics.notify(typedArray, index, count)
/// Wakes up agents waiting in the queue at a given position
fn atomics_notify<H: BuiltinHost>(_host: &mut H, _args: &[JSValue]) -> JSValue {
    // For now, return 0 since we don't have worker support yet
    make_number(0.0)
}

/// Atomics.waitAsync(typedArray, index, value, timeout)
/// Non-blocking async version of wait
fn atomics_wait_async<H: BuiltinHost>(host: &mut H, _args: &[JSValue]) -> JSValue {
    // Return a Promise-like object
    host.create_object()
}

/// Atomics.isLockFree(size)
/// Checks if the operation is atomic-free from locks
fn atomics_is_lock_free<H: BuiltinHost>(host: &mut H, args: &[JSValue]) -> JSValue {
    let size = args
        .first()
        .and_then(|v| to_i32(host.number_value(*v)))
        .unwrap_or(0) as usize;

    let is_lock_free = match size {
        1 | 2 | 4 => true,
        8 => false, // 64-bit may require locks
        _ => false,
    };

    JSValue::bool(is_lock_free)
}
