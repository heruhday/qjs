use std::collections::HashMap;

use value::{JSValue, make_undefined};

use crate::{BuiltinHost, install_global_function};

pub(crate) fn install<H: BuiltinHost>(host: &mut H, global_slots: &HashMap<&str, u16>) {
    let _ = install_global_function(host, global_slots, "eval", "__builtin_eval", &[]);
}

pub(crate) fn dispatch<H: BuiltinHost>(
    host: &mut H,
    name: &str,
    _callee_value: JSValue,
    _this_value: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    if name != "__builtin_eval" {
        return None;
    }

    let Some(source) = args
        .first()
        .and_then(|&value| host.string_text(value).map(str::to_owned))
    else {
        return Some(args.first().copied().unwrap_or_else(make_undefined));
    };

    Some(
        host.eval_source(&source)
            .unwrap_or_else(|_| make_undefined()),
    )
}
