use value::{JSValue, make_false, make_int32, make_number, make_true, to_f64};

use crate::ir::{IRBinaryOp, IRFunction, IRInst, IRUnaryOp, IRValue};
use crate::passes::Pass;

pub struct ConstantFolding;

impl Pass for ConstantFolding {
    fn name(&self) -> &'static str {
        "ConstantFolding"
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        let mut changed = false;

        for block in &mut ir.blocks {
            for inst in &mut block.instructions {
                let folded = match inst {
                    IRInst::Binary { dst, op, lhs, rhs } => match op {
                        IRBinaryOp::Add => {
                            fold_binary(dst.clone(), lhs.clone(), rhs.clone(), |l, r| l + r)
                        }
                        IRBinaryOp::Sub => {
                            fold_binary(dst.clone(), lhs.clone(), rhs.clone(), |l, r| l - r)
                        }
                        IRBinaryOp::Mul => {
                            fold_binary(dst.clone(), lhs.clone(), rhs.clone(), |l, r| l * r)
                        }
                        IRBinaryOp::Div => {
                            fold_binary(dst.clone(), lhs.clone(), rhs.clone(), |l, r| l / r)
                        }
                        IRBinaryOp::Eq => {
                            fold_compare(dst.clone(), lhs.clone(), rhs.clone(), |l, r| l == r)
                        }
                        IRBinaryOp::Lt => {
                            fold_compare(dst.clone(), lhs.clone(), rhs.clone(), |l, r| l < r)
                        }
                        IRBinaryOp::Lte => {
                            fold_compare(dst.clone(), lhs.clone(), rhs.clone(), |l, r| l <= r)
                        }
                        _ => None,
                    },
                    IRInst::Unary { dst, op, operand } => match op {
                        IRUnaryOp::Neg => {
                            fold_unary_numeric(dst.clone(), operand.clone(), |value| -value)
                        }
                        _ => None,
                    },
                    _ => None,
                };

                if let Some(replacement) = folded {
                    *inst = replacement;
                    changed = true;
                }
            }
        }

        changed
    }
}

fn fold_binary(
    dst: IRValue,
    lhs: IRValue,
    rhs: IRValue,
    op: impl Fn(f64, f64) -> f64,
) -> Option<IRInst> {
    let IRValue::Constant(lhs) = lhs else {
        return None;
    };
    let IRValue::Constant(rhs) = rhs else {
        return None;
    };

    let lhs = to_f64(lhs)?;
    let rhs = to_f64(rhs)?;
    let folded = op(lhs, rhs);

    Some(IRInst::LoadConst {
        dst,
        value: numeric_value(folded),
    })
}

fn fold_compare(
    dst: IRValue,
    lhs: IRValue,
    rhs: IRValue,
    op: impl Fn(f64, f64) -> bool,
) -> Option<IRInst> {
    let IRValue::Constant(lhs) = lhs else {
        return None;
    };
    let IRValue::Constant(rhs) = rhs else {
        return None;
    };

    let lhs = to_f64(lhs)?;
    let rhs = to_f64(rhs)?;
    Some(IRInst::LoadConst {
        dst,
        value: if op(lhs, rhs) {
            make_true()
        } else {
            make_false()
        },
    })
}

fn fold_unary_numeric(dst: IRValue, operand: IRValue, op: impl Fn(f64) -> f64) -> Option<IRInst> {
    let IRValue::Constant(operand) = operand else {
        return None;
    };

    let operand = to_f64(operand)?;
    Some(IRInst::LoadConst {
        dst,
        value: numeric_value(op(operand)),
    })
}

fn numeric_value(number: f64) -> JSValue {
    if number.fract() == 0.0 && number >= i32::MIN as f64 && number <= i32::MAX as f64 {
        make_int32(number as i32)
    } else {
        make_number(number)
    }
}
