use std::collections::{HashMap, VecDeque};

use cfg::{BlockId, CompareKind};
use value::{JSValue, bool_from_value, is_truthy, make_bool, make_int32, make_number, to_f64};

use crate::ir::{IRCondition, IRFunction, IRInst, IRTerminator, IRUnaryOp, IRValue};
use crate::passes::{CfgSimplification, Pass};

type Facts = HashMap<IRValue, JSValue>;
type EdgeFacts = HashMap<(BlockId, BlockId), Facts>;

pub struct SparseConditionalConstantPropagation;

impl Pass for SparseConditionalConstantPropagation {
    fn name(&self) -> &'static str {
        "SparseConditionalConstantPropagation"
    }

    fn is_structural(&self) -> bool {
        true
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        if ir.blocks.is_empty() || ir.entry >= ir.blocks.len() {
            return false;
        }

        let analysis = analyze_constants(ir);
        let mut changed = apply_simplifications(ir, &analysis);
        if changed {
            changed |= CfgSimplification.run(ir);
        }
        changed
    }
}

#[derive(Debug, Clone)]
struct AnalysisResult {
    edge_facts: EdgeFacts,
    before_instructions: Vec<Vec<Facts>>,
    before_terminators: Vec<Facts>,
    reachable: Vec<bool>,
}

fn analyze_constants(ir: &IRFunction) -> AnalysisResult {
    let mut edge_facts = EdgeFacts::new();
    let mut queued = vec![false; ir.blocks.len()];
    let mut worklist = VecDeque::new();
    worklist.push_back(ir.entry);
    queued[ir.entry] = true;

    while let Some(block_id) = worklist.pop_front() {
        queued[block_id] = false;
        let (entry_facts, reachable) = block_entry_facts(ir, block_id, &edge_facts);
        if !reachable {
            continue;
        }

        let exit_facts = transfer_block(ir, block_id, &entry_facts, &edge_facts);
        for (successor, successor_facts) in successor_edge_facts(
            &ir.blocks[block_id].terminator,
            &exit_facts,
            &ir.blocks[block_id].successors,
        ) {
            let edge = (block_id, successor);
            let updated = match edge_facts.get(&edge) {
                Some(previous) => join_maps(previous, &successor_facts),
                None => successor_facts,
            };

            if edge_facts.get(&edge) != Some(&updated) {
                edge_facts.insert(edge, updated);
                if successor < queued.len() && !queued[successor] {
                    queued[successor] = true;
                    worklist.push_back(successor);
                }
            }
        }
    }

    summarize_analysis(ir, edge_facts)
}

fn summarize_analysis(ir: &IRFunction, edge_facts: EdgeFacts) -> AnalysisResult {
    let mut before_instructions = Vec::with_capacity(ir.blocks.len());
    let mut before_terminators = Vec::with_capacity(ir.blocks.len());
    let mut reachable = Vec::with_capacity(ir.blocks.len());

    for block_id in 0..ir.blocks.len() {
        let (entry, is_reachable) = block_entry_facts(ir, block_id, &edge_facts);
        let mut current = entry;
        let mut block_before = Vec::with_capacity(ir.blocks[block_id].instructions.len());

        if is_reachable {
            for inst in &ir.blocks[block_id].instructions {
                block_before.push(current.clone());
                apply_instruction_facts(block_id, inst, &mut current, &edge_facts);
            }
        }

        before_instructions.push(block_before);
        before_terminators.push(current);
        reachable.push(is_reachable);
    }

    AnalysisResult {
        edge_facts,
        before_instructions,
        before_terminators,
        reachable,
    }
}

fn block_entry_facts(ir: &IRFunction, block_id: BlockId, edge_facts: &EdgeFacts) -> (Facts, bool) {
    if block_id == ir.entry {
        return (Facts::new(), true);
    }

    let mut incoming = ir.blocks[block_id]
        .predecessors
        .iter()
        .filter_map(|pred| edge_facts.get(&(*pred, block_id)));

    let Some(first) = incoming.next() else {
        return (Facts::new(), false);
    };

    let mut joined = first.clone();
    for facts in incoming {
        joined = join_maps(&joined, facts);
    }
    (joined, true)
}

fn transfer_block(
    ir: &IRFunction,
    block_id: BlockId,
    entry_facts: &Facts,
    edge_facts: &EdgeFacts,
) -> Facts {
    let mut current = entry_facts.clone();
    for inst in &ir.blocks[block_id].instructions {
        apply_instruction_facts(block_id, inst, &mut current, edge_facts);
    }
    current
}

fn apply_instruction_facts(
    block_id: BlockId,
    inst: &IRInst,
    current: &mut Facts,
    edge_facts: &EdgeFacts,
) {
    match inst {
        IRInst::Bytecode { defs, .. } => {
            for def in defs {
                current.remove(def);
            }
        }
        IRInst::Nop => {}
        _ => {
            let Some(dst) = defined_value(inst) else {
                return;
            };

            if let Some(value) = infer_instruction_constant(block_id, inst, current, edge_facts) {
                current.insert(dst, value);
            } else {
                current.remove(&dst);
            }
        }
    }
}

fn infer_instruction_constant(
    block_id: BlockId,
    inst: &IRInst,
    current: &Facts,
    edge_facts: &EdgeFacts,
) -> Option<JSValue> {
    match inst {
        IRInst::Phi { incoming, .. } => {
            let mut incoming_values = incoming.iter().filter_map(|(pred, value)| {
                edge_facts
                    .get(&(*pred, block_id))
                    .and_then(|facts| constant_for_value(value, facts))
            });

            let first = incoming_values.next()?;
            if incoming_values.all(|value| value == first) {
                Some(first)
            } else {
                None
            }
        }
        IRInst::Mov { src, .. } => constant_for_value(src, current),
        IRInst::LoadConst { value, .. } => Some(*value),
        IRInst::Unary { op, operand, .. } => infer_unary_constant(*op, operand, current),
        IRInst::Binary { op, lhs, rhs, .. } => infer_binary_constant(*op, lhs, rhs, current),
        IRInst::Bytecode { .. } | IRInst::Nop => None,
    }
}

fn successor_edge_facts(
    terminator: &IRTerminator,
    current: &Facts,
    successors: &[BlockId],
) -> Vec<(BlockId, Facts)> {
    match terminator {
        IRTerminator::None
        | IRTerminator::Return { .. }
        | IRTerminator::Throw { .. }
        | IRTerminator::TailCall { .. }
        | IRTerminator::CallReturn { .. } => Vec::new(),
        IRTerminator::Jump { target } => vec![(*target, current.clone())],
        IRTerminator::Branch {
            condition,
            target,
            fallthrough,
        } => match evaluate_condition(condition, current) {
            Some(true) => vec![(*target, current.clone())],
            Some(false) => vec![(*fallthrough, current.clone())],
            None => vec![(*target, current.clone()), (*fallthrough, current.clone())],
        },
        IRTerminator::Switch {
            key,
            cases,
            default_target,
        } => {
            if let Some(value) = constant_for_value(key, current) {
                let target = cases
                    .iter()
                    .find(|(case, _)| *case == value)
                    .map(|(_, target)| *target)
                    .unwrap_or(*default_target);
                return vec![(target, current.clone())];
            }

            let mut lowered = Vec::new();
            for &successor in successors {
                push_unique_successor(&mut lowered, successor, current.clone());
            }
            lowered
        }
        IRTerminator::Try {
            handler,
            fallthrough,
        } => vec![(*handler, current.clone()), (*fallthrough, current.clone())],
        IRTerminator::ConditionalReturn {
            condition,
            fallthrough,
            ..
        } => match evaluate_condition(condition, current) {
            Some(true) => Vec::new(),
            Some(false) | None => vec![(*fallthrough, current.clone())],
        },
    }
}

fn apply_simplifications(ir: &mut IRFunction, analysis: &AnalysisResult) -> bool {
    let mut changed = false;

    for block_id in 0..ir.blocks.len() {
        if !analysis.reachable[block_id] {
            continue;
        }

        for (index, inst) in ir.blocks[block_id].instructions.iter_mut().enumerate() {
            let current = &analysis.before_instructions[block_id][index];
            if let Some(replacement) =
                simplify_instruction(block_id, inst, current, &analysis.edge_facts)
            {
                *inst = replacement;
                changed = true;
            }
        }

        let current = &analysis.before_terminators[block_id];
        if simplify_terminator(&mut ir.blocks[block_id].terminator, current) {
            changed = true;
        }
    }

    changed
}

fn simplify_instruction(
    block_id: BlockId,
    inst: &IRInst,
    current: &Facts,
    edge_facts: &EdgeFacts,
) -> Option<IRInst> {
    let dst = defined_value(inst)?;
    let value = infer_instruction_constant(block_id, inst, current, edge_facts)?;

    match inst {
        IRInst::LoadConst { value: current, .. } if *current == value => None,
        _ => Some(IRInst::LoadConst { dst, value }),
    }
}

fn simplify_terminator(terminator: &mut IRTerminator, current: &Facts) -> bool {
    match terminator {
        IRTerminator::Branch {
            condition,
            target,
            fallthrough,
        } => match evaluate_condition(condition, current) {
            Some(true) => {
                *terminator = IRTerminator::Jump { target: *target };
                true
            }
            Some(false) => {
                *terminator = IRTerminator::Jump {
                    target: *fallthrough,
                };
                true
            }
            None => false,
        },
        IRTerminator::Switch {
            key,
            cases,
            default_target,
        } => {
            let Some(value) = constant_for_value(key, current) else {
                return false;
            };

            let target = cases
                .iter()
                .find(|(case, _)| *case == value)
                .map(|(_, target)| *target)
                .unwrap_or(*default_target);
            *terminator = IRTerminator::Jump { target };
            true
        }
        IRTerminator::ConditionalReturn {
            condition,
            value,
            fallthrough,
        } => match evaluate_condition(condition, current) {
            Some(true) => {
                *terminator = IRTerminator::Return {
                    value: Some(value.clone()),
                };
                true
            }
            Some(false) => {
                *terminator = IRTerminator::Jump {
                    target: *fallthrough,
                };
                true
            }
            None => false,
        },
        IRTerminator::None
        | IRTerminator::Jump { .. }
        | IRTerminator::Return { .. }
        | IRTerminator::Throw { .. }
        | IRTerminator::TailCall { .. }
        | IRTerminator::CallReturn { .. }
        | IRTerminator::Try { .. } => false,
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

fn infer_unary_constant(op: IRUnaryOp, operand: &IRValue, current: &Facts) -> Option<JSValue> {
    let operand = constant_for_value(operand, current)?;
    match op {
        IRUnaryOp::Neg => Some(numeric_value(-to_f64(operand)?)),
        IRUnaryOp::Inc => Some(numeric_value(to_f64(operand)? + 1.0)),
        IRUnaryOp::Dec => Some(numeric_value(to_f64(operand)? - 1.0)),
        IRUnaryOp::IsUndef => Some(make_bool(operand.is_undefined())),
        IRUnaryOp::IsNull => Some(make_bool(operand.is_null())),
        _ => None,
    }
}

fn infer_binary_constant(
    op: crate::ir::IRBinaryOp,
    lhs: &IRValue,
    rhs: &IRValue,
    current: &Facts,
) -> Option<JSValue> {
    use crate::ir::IRBinaryOp;

    match op {
        IRBinaryOp::Add => fold_numeric(lhs, rhs, current, |lhs, rhs| lhs + rhs),
        IRBinaryOp::Sub => fold_numeric(lhs, rhs, current, |lhs, rhs| lhs - rhs),
        IRBinaryOp::Mul => fold_numeric(lhs, rhs, current, |lhs, rhs| lhs * rhs),
        IRBinaryOp::Div => fold_numeric(lhs, rhs, current, |lhs, rhs| lhs / rhs),
        IRBinaryOp::Eq => compare_constant(CompareKind::Eq, lhs, rhs, current),
        IRBinaryOp::Lt => compare_constant(CompareKind::Lt, lhs, rhs, current),
        IRBinaryOp::Lte => compare_constant(CompareKind::Lte, lhs, rhs, current),
        IRBinaryOp::StrictEq => {
            let lhs = constant_for_value(lhs, current)?;
            let rhs = constant_for_value(rhs, current)?;
            Some(make_bool(lhs == rhs))
        }
        IRBinaryOp::StrictNeq => {
            let lhs = constant_for_value(lhs, current)?;
            let rhs = constant_for_value(rhs, current)?;
            Some(make_bool(lhs != rhs))
        }
        _ => None,
    }
}

fn compare_constant(
    kind: CompareKind,
    lhs: &IRValue,
    rhs: &IRValue,
    current: &Facts,
) -> Option<JSValue> {
    let lhs = to_f64(constant_for_value(lhs, current)?)?;
    let rhs = to_f64(constant_for_value(rhs, current)?)?;
    let value = match kind {
        CompareKind::Eq => lhs == rhs,
        CompareKind::Lt => lhs < rhs,
        CompareKind::Lte => lhs <= rhs,
    };
    Some(make_bool(value))
}

fn fold_numeric(
    lhs: &IRValue,
    rhs: &IRValue,
    current: &Facts,
    op: impl Fn(f64, f64) -> f64,
) -> Option<JSValue> {
    let lhs = to_f64(constant_for_value(lhs, current)?)?;
    let rhs = to_f64(constant_for_value(rhs, current)?)?;
    Some(numeric_value(op(lhs, rhs)))
}

fn evaluate_condition(condition: &IRCondition, current: &Facts) -> Option<bool> {
    match condition {
        IRCondition::Truthy { value, negate } => {
            let truthy = is_truthy(constant_for_value(value, current)?);
            Some(truthy ^ *negate)
        }
        IRCondition::Compare {
            kind,
            lhs,
            rhs,
            negate,
        } => {
            let truth = compare_constant(*kind, lhs, rhs, current).and_then(bool_from_value)?;
            Some(truth ^ *negate)
        }
    }
}

fn constant_for_value(value: &IRValue, current: &Facts) -> Option<JSValue> {
    match value {
        IRValue::Register(_, _) => current.get(value).copied(),
        IRValue::Constant(value) => Some(*value),
    }
}

fn join_maps(left: &Facts, right: &Facts) -> Facts {
    let mut joined = Facts::new();

    for (key, left_value) in left {
        let Some(right_value) = right.get(key) else {
            continue;
        };
        if left_value == right_value {
            joined.insert(key.clone(), *left_value);
        }
    }

    joined
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

fn push_unique_successor(edges: &mut Vec<(BlockId, Facts)>, block: BlockId, facts: Facts) {
    if edges.iter().any(|(candidate, _)| *candidate == block) {
        return;
    }
    edges.push((block, facts));
}
