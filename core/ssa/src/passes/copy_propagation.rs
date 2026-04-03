use std::collections::{HashMap, HashSet};

use crate::ir::{IRCondition, IRFunction, IRInst, IRTerminator, IRValue};
use crate::passes::Pass;

pub struct CopyPropagation;

impl Pass for CopyPropagation {
    fn name(&self) -> &'static str {
        "CopyPropagation"
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        // Check if the function contains any loops (blocks with backedges)
        let has_loops = ir.blocks.iter().any(|b| b.successors.contains(&b.id));
        
        // Skip copy propagation if there are loops, as it can break loop structures
        if has_loops {
            return false;
        }

        let mut changed = false;
        let mut copies = HashMap::<IRValue, IRValue>::new();

        for block in &mut ir.blocks {
            for inst in &mut block.instructions {
                rewrite_instruction(inst, &copies, &mut changed);

                match inst {
                    IRInst::Mov { dst, src } => {
                        invalidate_copies(&mut copies, dst);
                        if dst != src {
                            copies.insert(dst.clone(), src.clone());
                        }
                    }
                    IRInst::Phi { dst, .. }
                    | IRInst::LoadConst { dst, .. }
                    | IRInst::Unary { dst, .. }
                    | IRInst::Binary { dst, .. } => {
                        invalidate_copies(&mut copies, dst);
                    }
                    IRInst::Bytecode { defs, .. } => {
                        for def in defs {
                            invalidate_copies(&mut copies, def);
                        }
                    }
                    IRInst::Nop => {}
                }
            }

            rewrite_terminator(&mut block.terminator, &copies, &mut changed);
        }

        changed
    }
}

fn rewrite_instruction(inst: &mut IRInst, copies: &HashMap<IRValue, IRValue>, changed: &mut bool) {
    match inst {
        IRInst::Phi { incoming, .. } => {
            for (_, value) in incoming {
                let rewritten = resolve_copy(value.clone(), copies);
                if *value != rewritten {
                    *value = rewritten;
                    *changed = true;
                }
            }
        }
        IRInst::Mov { src, .. } => {
            let rewritten = resolve_copy(src.clone(), copies);
            if *src != rewritten {
                *src = rewritten;
                *changed = true;
            }
        }
        IRInst::Unary { operand, .. } => {
            let rewritten = resolve_copy(operand.clone(), copies);
            if *operand != rewritten {
                *operand = rewritten;
                *changed = true;
            }
        }
        IRInst::Binary { lhs, rhs, .. } => {
            let rewritten_lhs = resolve_copy(lhs.clone(), copies);
            let rewritten_rhs = resolve_copy(rhs.clone(), copies);
            if *lhs != rewritten_lhs {
                *lhs = rewritten_lhs;
                *changed = true;
            }
            if *rhs != rewritten_rhs {
                *rhs = rewritten_rhs;
                *changed = true;
            }
        }
        IRInst::Bytecode { uses, .. } => {
            for value in uses {
                let rewritten = resolve_copy(value.clone(), copies);
                if *value != rewritten {
                    *value = rewritten;
                    *changed = true;
                }
            }
        }
        IRInst::LoadConst { .. } | IRInst::Nop => {}
    }
}

fn rewrite_terminator(
    terminator: &mut IRTerminator,
    copies: &HashMap<IRValue, IRValue>,
    changed: &mut bool,
) {
    match terminator {
        IRTerminator::Branch { condition, .. }
        | IRTerminator::ConditionalReturn { condition, .. } => {
            rewrite_condition(condition, copies, changed)
        }
        IRTerminator::Switch { key, .. }
        | IRTerminator::Throw { value: key }
        | IRTerminator::TailCall { callee: key, .. }
        | IRTerminator::CallReturn { callee: key, .. } => {
            let rewritten = resolve_copy(key.clone(), copies);
            if *key != rewritten {
                *key = rewritten;
                *changed = true;
            }
        }
        IRTerminator::Return { value } => {
            if let Some(current) = value {
                let rewritten = resolve_copy(current.clone(), copies);
                if *current != rewritten {
                    *current = rewritten;
                    *changed = true;
                }
            }
        }
        IRTerminator::Jump { .. } | IRTerminator::Try { .. } | IRTerminator::None => {}
    }
}

fn rewrite_condition(
    condition: &mut IRCondition,
    copies: &HashMap<IRValue, IRValue>,
    changed: &mut bool,
) {
    match condition {
        IRCondition::Truthy { value, .. } => {
            let rewritten = resolve_copy(value.clone(), copies);
            if *value != rewritten {
                *value = rewritten;
                *changed = true;
            }
        }
        IRCondition::Compare { lhs, rhs, .. } => {
            let rewritten_lhs = resolve_copy(lhs.clone(), copies);
            let rewritten_rhs = resolve_copy(rhs.clone(), copies);
            if *lhs != rewritten_lhs {
                *lhs = rewritten_lhs;
                *changed = true;
            }
            if *rhs != rewritten_rhs {
                *rhs = rewritten_rhs;
                *changed = true;
            }
        }
    }
}

fn resolve_copy(mut value: IRValue, copies: &HashMap<IRValue, IRValue>) -> IRValue {
    let mut seen = HashSet::new();

    while let Some(next) = copies.get(&value) {
        if !seen.insert(value.clone()) {
            break;
        }
        value = next.clone();
    }

    value
}

fn invalidate_copies(copies: &mut HashMap<IRValue, IRValue>, defined: &IRValue) {
    let snapshot = copies.clone();
    copies.retain(|dst, src| dst != defined && resolve_copy(src.clone(), &snapshot) != *defined);
}
