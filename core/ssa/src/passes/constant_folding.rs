use crate::ir::{IRBinaryOp, IRFunction, IRInst, IRUnaryOp, IRValue};
use crate::passes::Pass;

use super::constant_eval::{fold_binary_constant, fold_unary_constant};

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
                    IRInst::Binary { dst, op, lhs, rhs } => {
                        fold_binary_inst(dst.clone(), *op, lhs.clone(), rhs.clone())
                    }
                    IRInst::Unary { dst, op, operand } => {
                        fold_unary_inst(dst.clone(), *op, operand.clone())
                    }
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

fn fold_binary_inst(dst: IRValue, op: IRBinaryOp, lhs: IRValue, rhs: IRValue) -> Option<IRInst> {
    let IRValue::Constant(lhs) = lhs else {
        return None;
    };
    let IRValue::Constant(rhs) = rhs else {
        return None;
    };

    Some(IRInst::LoadConst {
        dst,
        value: fold_binary_constant(op, lhs, rhs)?,
    })
}

fn fold_unary_inst(dst: IRValue, op: IRUnaryOp, operand: IRValue) -> Option<IRInst> {
    let IRValue::Constant(operand) = operand else {
        return None;
    };

    Some(IRInst::LoadConst {
        dst,
        value: fold_unary_constant(op, operand)?,
    })
}
