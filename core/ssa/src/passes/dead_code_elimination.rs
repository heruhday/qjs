use std::collections::HashSet;

use crate::ir::{IRCondition, IRFunction, IRInst, IRTerminator, IRValue};
use crate::passes::Pass;

pub struct DeadCodeElimination;

impl Pass for DeadCodeElimination {
    fn name(&self) -> &'static str {
        "DeadCodeElimination"
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        let mut changed = false;
        let mut live = HashSet::<IRValue>::new();

        for block in &ir.blocks {
            collect_terminator_uses(&block.terminator, &mut live);
        }

        for block in ir.blocks.iter_mut().rev() {
            let mut retained = Vec::with_capacity(block.instructions.len());

            for inst in block.instructions.iter().rev() {
                let Some(dst) = defined_value(inst) else {
                    retained.push(inst.clone());
                    continue;
                };

                if live.contains(&dst) {
                    collect_instruction_uses(inst, &mut live);
                    retained.push(inst.clone());
                } else if !matches!(inst, IRInst::Nop) {
                    changed = true;
                }
            }

            retained.reverse();
            if block.instructions.len() != retained.len() {
                changed = true;
            }
            block.instructions = retained;
        }

        changed
    }
}

fn defined_value(inst: &IRInst) -> Option<IRValue> {
    match inst {
        IRInst::Phi { dst, .. }
        | IRInst::Mov { dst, .. }
        | IRInst::LoadConst { dst, .. }
        | IRInst::Unary { dst, .. }
        | IRInst::Binary { dst, .. } => Some(dst.clone()),
        IRInst::Bytecode { .. } | IRInst::Nop => None,
    }
}

fn collect_instruction_uses(inst: &IRInst, live: &mut HashSet<IRValue>) {
    match inst {
        IRInst::Phi { incoming, .. } => {
            for (_, value) in incoming {
                live.insert(value.clone());
            }
        }
        IRInst::Mov { src, .. } => {
            live.insert(src.clone());
        }
        IRInst::Unary { operand, .. } => {
            live.insert(operand.clone());
        }
        IRInst::Binary { lhs, rhs, .. } => {
            live.insert(lhs.clone());
            live.insert(rhs.clone());
        }
        IRInst::Bytecode { uses, .. } => {
            for value in uses {
                live.insert(value.clone());
            }
        }
        IRInst::LoadConst { .. } | IRInst::Nop => {}
    }
}

fn collect_terminator_uses(terminator: &IRTerminator, live: &mut HashSet<IRValue>) {
    match terminator {
        IRTerminator::Branch { condition, .. }
        | IRTerminator::ConditionalReturn { condition, .. } => {
            collect_condition_uses(condition, live)
        }
        IRTerminator::Switch { key, .. }
        | IRTerminator::Throw { value: key }
        | IRTerminator::TailCall { callee: key, .. }
        | IRTerminator::CallReturn { callee: key, .. } => {
            live.insert(key.clone());
        }
        IRTerminator::Return { value } => {
            if let Some(value) = value {
                live.insert(value.clone());
            }
        }
        IRTerminator::Jump { .. } | IRTerminator::Try { .. } | IRTerminator::None => {}
    }
}

fn collect_condition_uses(condition: &IRCondition, live: &mut HashSet<IRValue>) {
    match condition {
        IRCondition::Truthy { value, .. } => {
            live.insert(value.clone());
        }
        IRCondition::Compare { lhs, rhs, .. } => {
            live.insert(lhs.clone());
            live.insert(rhs.clone());
        }
    }
}
