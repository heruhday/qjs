use cfg::CompareKind;
use value::{JSValue, make_bool, make_int32, make_number};

use crate::ir::{IRBinaryOp, IRUnaryOp};

pub(super) fn fold_unary_constant(op: IRUnaryOp, operand: JSValue) -> Option<JSValue> {
    match op {
        IRUnaryOp::Neg => fold_numeric_unary(operand, |value| -value),
        IRUnaryOp::Inc => fold_numeric_unary(operand, |value| value + 1.0),
        IRUnaryOp::Dec => fold_numeric_unary(operand, |value| value - 1.0),
        IRUnaryOp::ToNum => {
            can_fold_to_number(operand).then(|| numeric_value(operand.to_number_ecma()))
        }
        IRUnaryOp::IsUndef => Some(make_bool(operand.is_undefined())),
        IRUnaryOp::IsNull => Some(make_bool(operand.is_null())),
        IRUnaryOp::BitNot => {
            can_fold_to_number(operand).then(|| make_int32(!operand.to_i32_ecma()))
        }
        _ => None,
    }
}

pub(super) fn fold_binary_constant(op: IRBinaryOp, lhs: JSValue, rhs: JSValue) -> Option<JSValue> {
    match op {
        IRBinaryOp::Add => fold_numeric_binary(lhs, rhs, |left, right| left + right),
        IRBinaryOp::Sub => fold_numeric_binary(lhs, rhs, |left, right| left - right),
        IRBinaryOp::Mul => fold_numeric_binary(lhs, rhs, |left, right| left * right),
        IRBinaryOp::Div => fold_numeric_binary(lhs, rhs, |left, right| left / right),
        IRBinaryOp::Mod => fold_numeric_binary(lhs, rhs, |left, right| left % right),
        IRBinaryOp::Pow => fold_numeric_binary(lhs, rhs, |left, right| left.powf(right)),
        IRBinaryOp::Eq => evaluate_compare(CompareKind::Eq, lhs, rhs).map(make_bool),
        IRBinaryOp::Lt => evaluate_compare(CompareKind::Lt, lhs, rhs).map(make_bool),
        IRBinaryOp::Lte => evaluate_compare(CompareKind::Lte, lhs, rhs).map(make_bool),
        IRBinaryOp::StrictEq => evaluate_strict_eq(lhs, rhs).map(make_bool),
        IRBinaryOp::StrictNeq => evaluate_strict_eq(lhs, rhs).map(|value| make_bool(!value)),
        IRBinaryOp::BitAnd => fold_i32_binary(lhs, rhs, |left, right| make_int32(left & right)),
        IRBinaryOp::BitOr => fold_i32_binary(lhs, rhs, |left, right| make_int32(left | right)),
        IRBinaryOp::BitXor => fold_i32_binary(lhs, rhs, |left, right| make_int32(left ^ right)),
        IRBinaryOp::Shl => {
            fold_i32_binary(lhs, rhs, |left, right| make_int32(left << (right & 31)))
        }
        IRBinaryOp::Shr => {
            fold_i32_binary(lhs, rhs, |left, right| make_int32(left >> (right & 31)))
        }
        IRBinaryOp::Ushr => fold_i32_binary(lhs, rhs, |left, right| {
            let shifted = (left as u32) >> ((right & 31) as u32);
            make_number(shifted as f64)
        }),
        IRBinaryOp::LogicalAnd => Some(if lhs.is_truthy() { rhs } else { lhs }),
        IRBinaryOp::LogicalOr => Some(if lhs.is_truthy() { lhs } else { rhs }),
        IRBinaryOp::NullishCoalesce => Some(if lhs.is_null() || lhs.is_undefined() {
            rhs
        } else {
            lhs
        }),
        IRBinaryOp::In => (!rhs.is_object()).then(|| make_bool(false)),
        IRBinaryOp::Instanceof => (!lhs.is_object()).then(|| make_bool(false)),
        IRBinaryOp::AddStr => None,
    }
}

pub(super) fn evaluate_compare(kind: CompareKind, lhs: JSValue, rhs: JSValue) -> Option<bool> {
    match kind {
        CompareKind::Eq => evaluate_abstract_eq(lhs, rhs),
        CompareKind::Neq => evaluate_abstract_eq(lhs, rhs).map(|value| !value),
        CompareKind::Lt => evaluate_relational_compare(lhs, rhs, RelationalOp::Lt),
        CompareKind::Lte => evaluate_relational_compare(lhs, rhs, RelationalOp::Lte),
        CompareKind::LteFalse => evaluate_relational_compare(lhs, rhs, RelationalOp::LteFalse),
    }
}

#[derive(Clone, Copy)]
enum RelationalOp {
    Lt,
    Lte,
    LteFalse,
}

fn evaluate_relational_compare(lhs: JSValue, rhs: JSValue, op: RelationalOp) -> Option<bool> {
    if lhs.is_string() || rhs.is_string() {
        if lhs == rhs && lhs.is_string() && rhs.is_string() {
            return Some(matches!(op, RelationalOp::Lte));
        }
        return None;
    }

    if !(can_fold_to_number(lhs) && can_fold_to_number(rhs)) {
        return None;
    }

    let left = lhs.to_number_ecma();
    let right = rhs.to_number_ecma();
    let result = match op {
        RelationalOp::Lt => !left.is_nan() && !right.is_nan() && left < right,
        RelationalOp::Lte => {
            (!left.is_nan() && !right.is_nan() && left < right)
                || evaluate_strict_eq(lhs, rhs).unwrap_or(false)
        }
        RelationalOp::LteFalse => {
            let less_or_equal = (!left.is_nan() && !right.is_nan() && left < right)
                || evaluate_strict_eq(lhs, rhs).unwrap_or(false);
            !less_or_equal
        }
    };

    Some(result)
}

fn evaluate_abstract_eq(lhs: JSValue, rhs: JSValue) -> Option<bool> {
    if let Some(strict) = evaluate_strict_eq(lhs, rhs)
        && strict
    {
        return Some(true);
    }

    if is_nullish(lhs) && is_nullish(rhs) {
        return Some(true);
    }

    if lhs.is_object() || rhs.is_object() {
        if is_nullish(lhs) || is_nullish(rhs) {
            return Some(false);
        }
        return None;
    }

    if lhs.as_bool().is_some() && can_fold_to_number(rhs) && !rhs.is_string() {
        return Some(lhs.to_number_ecma() == rhs.to_number_ecma());
    }
    if rhs.as_bool().is_some() && can_fold_to_number(lhs) && !lhs.is_string() {
        return Some(lhs.to_number_ecma() == rhs.to_number_ecma());
    }

    if is_numeric_primitive(lhs) && is_numeric_primitive(rhs) {
        let left = lhs.to_number_ecma();
        let right = rhs.to_number_ecma();
        return Some(!left.is_nan() && !right.is_nan() && left == right);
    }

    if is_nullish(lhs) || is_nullish(rhs) {
        return Some(false);
    }

    None
}

fn evaluate_strict_eq(lhs: JSValue, rhs: JSValue) -> Option<bool> {
    if lhs == rhs {
        if is_numeric_primitive(lhs) {
            return Some(!lhs.to_number_ecma().is_nan());
        }
        return Some(true);
    }

    if lhs.is_object() && rhs.is_object() {
        return Some(false);
    }

    if lhs.as_bool().is_some() && rhs.as_bool().is_some() {
        return Some(false);
    }

    if is_numeric_primitive(lhs) && is_numeric_primitive(rhs) {
        let left = lhs.to_number_ecma();
        let right = rhs.to_number_ecma();
        return Some(!left.is_nan() && !right.is_nan() && left == right);
    }

    if lhs.is_null() && rhs.is_null() {
        return Some(true);
    }
    if lhs.is_undefined() && rhs.is_undefined() {
        return Some(true);
    }

    None
}

fn fold_numeric_unary(operand: JSValue, op: impl FnOnce(f64) -> f64) -> Option<JSValue> {
    can_fold_to_number(operand).then(|| numeric_value(op(operand.to_number_ecma())))
}

fn fold_numeric_binary(
    lhs: JSValue,
    rhs: JSValue,
    op: impl FnOnce(f64, f64) -> f64,
) -> Option<JSValue> {
    if !(can_fold_to_number(lhs) && can_fold_to_number(rhs)) {
        return None;
    }
    Some(numeric_value(op(
        lhs.to_number_ecma(),
        rhs.to_number_ecma(),
    )))
}

fn fold_i32_binary(
    lhs: JSValue,
    rhs: JSValue,
    op: impl FnOnce(i32, i32) -> JSValue,
) -> Option<JSValue> {
    if !(can_fold_to_number(lhs) && can_fold_to_number(rhs)) {
        return None;
    }
    Some(op(lhs.to_i32_ecma(), rhs.to_i32_ecma()))
}

fn can_fold_to_number(value: JSValue) -> bool {
    !value.is_string() && !value.is_heap()
}

fn is_nullish(value: JSValue) -> bool {
    value.is_null() || value.is_undefined()
}

fn is_numeric_primitive(value: JSValue) -> bool {
    value.as_i32().is_some() || value.as_f64().is_some()
}

fn numeric_value(number: f64) -> JSValue {
    if number.is_finite()
        && number.fract() == 0.0
        && number >= i32::MIN as f64
        && number <= i32::MAX as f64
    {
        make_int32(number as i32)
    } else {
        make_number(number)
    }
}
