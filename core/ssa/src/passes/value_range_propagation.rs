use std::collections::{HashMap, VecDeque};

use cfg::{BlockId, CompareKind};
use value::{JSValue, bool_from_value, is_truthy, make_bool, make_int32, make_number, to_f64};

use crate::ir::{IRCondition, IRFunction, IRInst, IRTerminator, IRUnaryOp, IRValue};
use crate::passes::Pass;

type Facts = HashMap<IRValue, Fact>;
type EdgeFacts = HashMap<(BlockId, BlockId), Facts>;

pub struct ValueRangePropagation;

impl Pass for ValueRangePropagation {
    fn name(&self) -> &'static str {
        "ValueRangePropagation"
    }

    fn is_structural(&self) -> bool {
        true
    }

    fn run(&self, ir: &mut IRFunction) -> bool {
        if ir.blocks.is_empty() || ir.entry >= ir.blocks.len() {
            return false;
        }

        let analysis = analyze_ranges(ir);
        let changed = apply_simplifications(ir, &analysis);
        if changed {
            recompute_cfg_links(ir);
        }
        changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Fact {
    Constant(JSValue),
    Range(IntRange),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IntRange {
    min: i64,
    max: i64,
}

#[derive(Debug, Clone)]
struct AnalysisResult {
    edge_facts: EdgeFacts,
    before_instructions: Vec<Vec<Facts>>,
    before_terminators: Vec<Facts>,
    reachable: Vec<bool>,
}

fn analyze_ranges(ir: &IRFunction) -> AnalysisResult {
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
                Some(previous) => widen_maps(previous, &successor_facts),
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
                apply_instruction_facts(
                    block_id,
                    &ir.blocks[block_id].predecessors,
                    inst,
                    &mut current,
                    &edge_facts,
                );
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
        apply_instruction_facts(
            block_id,
            &ir.blocks[block_id].predecessors,
            inst,
            &mut current,
            edge_facts,
        );
    }
    current
}

fn apply_instruction_facts(
    block_id: BlockId,
    predecessors: &[BlockId],
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

            let inferred =
                infer_instruction_fact(block_id, predecessors, inst, current, edge_facts);
            if let Some(fact) = inferred {
                current.insert(dst, fact);
            } else {
                current.remove(&dst);
            }
        }
    }
}

fn infer_instruction_fact(
    block_id: BlockId,
    predecessors: &[BlockId],
    inst: &IRInst,
    current: &Facts,
    edge_facts: &EdgeFacts,
) -> Option<Fact> {
    match inst {
        IRInst::Phi { incoming, .. } => {
            let mut incoming_facts = incoming.iter().filter_map(|(pred, value)| {
                edge_facts
                    .get(&(*pred, block_id))
                    .map(|facts| fact_for_value(value, facts))
            });

            let Some(first) = incoming_facts.next().flatten() else {
                if predecessors.is_empty() {
                    return None;
                }
                return None;
            };

            let mut joined = first;
            for maybe_fact in incoming_facts {
                let fact = maybe_fact?;
                joined = join_fact(&joined, &fact)?;
            }
            Some(joined)
        }
        IRInst::Mov { src, .. } => fact_for_value(src, current),
        IRInst::LoadConst { value, .. } => Some(Fact::Constant(*value)),
        IRInst::Unary { op, operand, .. } => infer_unary_fact(*op, operand, current),
        IRInst::Binary { op, lhs, rhs, .. } => infer_binary_fact(*op, lhs, rhs, current),
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
            Some(true) => vec![(*target, refine_condition_facts(current, condition, true))],
            Some(false) => vec![(
                *fallthrough,
                refine_condition_facts(current, condition, false),
            )],
            None => vec![
                (*target, refine_condition_facts(current, condition, true)),
                (
                    *fallthrough,
                    refine_condition_facts(current, condition, false),
                ),
            ],
        },
        IRTerminator::Switch {
            key,
            cases,
            default_target,
        } => {
            if let Some(value) = fact_for_value(key, current).and_then(|fact| exact_constant(&fact))
            {
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
            Some(false) => vec![(
                *fallthrough,
                refine_condition_facts(current, condition, false),
            )],
            None => vec![(
                *fallthrough,
                refine_condition_facts(current, condition, false),
            )],
        },
    }
}

fn apply_simplifications(ir: &mut IRFunction, analysis: &AnalysisResult) -> bool {
    let mut changed = false;

    for block_id in 0..ir.blocks.len() {
        if !analysis.reachable[block_id] {
            continue;
        }

        let predecessors = ir.blocks[block_id].predecessors.clone();
        for (index, inst) in ir.blocks[block_id].instructions.iter_mut().enumerate() {
            let current = &analysis.before_instructions[block_id][index];
            if let Some(replacement) =
                simplify_instruction(block_id, &predecessors, inst, current, &analysis.edge_facts)
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
    predecessors: &[BlockId],
    inst: &IRInst,
    current: &Facts,
    edge_facts: &EdgeFacts,
) -> Option<IRInst> {
    let dst = defined_value(inst)?;
    let inferred = infer_instruction_fact(block_id, predecessors, inst, current, edge_facts)?;
    let value = exact_constant(&inferred)?;

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
            let Some(value) = fact_for_value(key, current).and_then(|fact| exact_constant(&fact))
            else {
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

fn infer_unary_fact(op: IRUnaryOp, operand: &IRValue, current: &Facts) -> Option<Fact> {
    let operand = fact_for_value(operand, current)?;
    match op {
        IRUnaryOp::Neg => {
            let range = int_range_for_fact(&operand)?;
            Some(range_to_fact(IntRange {
                min: range.max.checked_neg()?,
                max: range.min.checked_neg()?,
            }))
        }
        IRUnaryOp::IsUndef => {
            let value = exact_constant(&operand)?;
            Some(Fact::Constant(make_bool(value.is_undefined())))
        }
        IRUnaryOp::IsNull => {
            let value = exact_constant(&operand)?;
            Some(Fact::Constant(make_bool(value.is_null())))
        }
        _ => None,
    }
}

fn infer_add_fact(lhs: &IRValue, rhs: &IRValue, current: &Facts) -> Option<Fact> {
    let lhs = int_range_for_value(lhs, current)?;
    let rhs = int_range_for_value(rhs, current)?;
    Some(range_to_fact(IntRange {
        min: lhs.min.checked_add(rhs.min)?,
        max: lhs.max.checked_add(rhs.max)?,
    }))
}

fn infer_sub_fact(lhs: &IRValue, rhs: &IRValue, current: &Facts) -> Option<Fact> {
    let lhs = int_range_for_value(lhs, current)?;
    let rhs = int_range_for_value(rhs, current)?;
    Some(range_to_fact(IntRange {
        min: lhs.min.checked_sub(rhs.max)?,
        max: lhs.max.checked_sub(rhs.min)?,
    }))
}

fn infer_mul_fact(lhs: &IRValue, rhs: &IRValue, current: &Facts) -> Option<Fact> {
    let lhs = int_range_for_value(lhs, current)?;
    let rhs = int_range_for_value(rhs, current)?;
    let candidates = [
        lhs.min.checked_mul(rhs.min)?,
        lhs.min.checked_mul(rhs.max)?,
        lhs.max.checked_mul(rhs.min)?,
        lhs.max.checked_mul(rhs.max)?,
    ];
    let min = *candidates.iter().min()?;
    let max = *candidates.iter().max()?;
    Some(range_to_fact(IntRange { min, max }))
}

fn infer_binary_fact(
    op: crate::ir::IRBinaryOp,
    lhs: &IRValue,
    rhs: &IRValue,
    current: &Facts,
) -> Option<Fact> {
    use crate::ir::IRBinaryOp;

    match op {
        IRBinaryOp::Add => infer_add_fact(lhs, rhs, current),
        IRBinaryOp::Sub => infer_sub_fact(lhs, rhs, current),
        IRBinaryOp::Mul => infer_mul_fact(lhs, rhs, current),
        IRBinaryOp::Eq => compare_fact(CompareKind::Eq, lhs, rhs, current),
        IRBinaryOp::Lt => compare_fact(CompareKind::Lt, lhs, rhs, current),
        IRBinaryOp::Lte => compare_fact(CompareKind::Lte, lhs, rhs, current),
        IRBinaryOp::StrictEq => strict_compare_fact(lhs, rhs, current, false),
        IRBinaryOp::StrictNeq => strict_compare_fact(lhs, rhs, current, true),
        _ => None,
    }
}

fn compare_fact(kind: CompareKind, lhs: &IRValue, rhs: &IRValue, current: &Facts) -> Option<Fact> {
    if lhs == rhs {
        let result = matches!(kind, CompareKind::Eq | CompareKind::Lte);
        return Some(Fact::Constant(make_bool(result)));
    }

    let lhs_constant = fact_for_value(lhs, current).and_then(|fact| exact_constant(&fact));
    let rhs_constant = fact_for_value(rhs, current).and_then(|fact| exact_constant(&fact));
    if let (Some(lhs), Some(rhs)) = (lhs_constant, rhs_constant) {
        let lhs = to_f64(lhs)?;
        let rhs = to_f64(rhs)?;
        let result = match kind {
            CompareKind::Eq => lhs == rhs,
            CompareKind::Lt => lhs < rhs,
            CompareKind::Lte => lhs <= rhs,
        };
        return Some(Fact::Constant(make_bool(result)));
    }

    let lhs_range = int_range_for_value(lhs, current)?;
    let rhs_range = int_range_for_value(rhs, current)?;
    let result = match kind {
        CompareKind::Eq => {
            if lhs_range.max < rhs_range.min || rhs_range.max < lhs_range.min {
                Some(false)
            } else if lhs_range.min == lhs_range.max
                && rhs_range.min == rhs_range.max
                && lhs_range.min == rhs_range.min
            {
                Some(true)
            } else {
                None
            }
        }
        CompareKind::Lt => {
            if lhs_range.max < rhs_range.min {
                Some(true)
            } else if lhs_range.min >= rhs_range.max {
                Some(false)
            } else {
                None
            }
        }
        CompareKind::Lte => {
            if lhs_range.max <= rhs_range.min {
                Some(true)
            } else if lhs_range.min > rhs_range.max {
                Some(false)
            } else {
                None
            }
        }
    }?;

    Some(Fact::Constant(make_bool(result)))
}

fn strict_compare_fact(
    lhs: &IRValue,
    rhs: &IRValue,
    current: &Facts,
    negate: bool,
) -> Option<Fact> {
    if lhs == rhs {
        return Some(Fact::Constant(make_bool(!negate)));
    }

    let lhs = fact_for_value(lhs, current).and_then(|fact| exact_constant(&fact))?;
    let rhs = fact_for_value(rhs, current).and_then(|fact| exact_constant(&fact))?;
    Some(Fact::Constant(make_bool((lhs == rhs) ^ negate)))
}

fn evaluate_condition(condition: &IRCondition, current: &Facts) -> Option<bool> {
    match condition {
        IRCondition::Truthy { value, negate } => {
            let truthy = truthiness_of_value(value, current)?;
            Some(truthy ^ *negate)
        }
        IRCondition::Compare {
            kind,
            lhs,
            rhs,
            negate,
        } => {
            let truth = compare_fact(*kind, lhs, rhs, current)
                .and_then(|fact| exact_constant(&fact))
                .and_then(bool_from_value)?;
            Some(truth ^ *negate)
        }
    }
}

fn truthiness_of_value(value: &IRValue, current: &Facts) -> Option<bool> {
    let fact = fact_for_value(value, current)?;
    if let Some(constant) = exact_constant(&fact) {
        return Some(is_truthy(constant));
    }

    let range = int_range_for_fact(&fact)?;
    if range.min == 0 && range.max == 0 {
        Some(false)
    } else if range.max < 0 || range.min > 0 {
        Some(true)
    } else {
        None
    }
}

fn refine_condition_facts(current: &Facts, condition: &IRCondition, branch_taken: bool) -> Facts {
    let mut refined = current.clone();

    match condition {
        IRCondition::Truthy { value, negate } => {
            let want_truthy = branch_taken ^ *negate;
            refine_truthy_value(&mut refined, value, want_truthy);
        }
        IRCondition::Compare {
            kind,
            lhs,
            rhs,
            negate,
        } => {
            let want_compare = branch_taken ^ *negate;
            refine_compare(&mut refined, *kind, lhs, rhs, want_compare);
        }
    }

    refined
}

fn refine_truthy_value(facts: &mut Facts, value: &IRValue, want_truthy: bool) {
    if let Some(constant) = fact_for_value(value, facts).and_then(|fact| exact_constant(&fact)) {
        facts.insert(value.clone(), Fact::Constant(constant));
        return;
    }

    let Some(range) = int_range_for_value(value, facts) else {
        return;
    };

    if want_truthy {
        if range.min > 0 || range.max < 0 {
            constrain_register(facts, value, range);
        }
    } else if range.min == 0 && range.max == 0 {
        constrain_register(facts, value, range);
    }
}

fn refine_compare(
    facts: &mut Facts,
    kind: CompareKind,
    lhs: &IRValue,
    rhs: &IRValue,
    want_compare: bool,
) {
    if let Some(rhs_exact) = exact_int_for_value(rhs, facts) {
        refine_register_vs_const(facts, kind, lhs, rhs_exact, want_compare, false);
    }
    if let Some(lhs_exact) = exact_int_for_value(lhs, facts) {
        refine_register_vs_const(facts, kind, rhs, lhs_exact, want_compare, true);
    }

    if kind == CompareKind::Eq && want_compare {
        let lhs_range = int_range_for_value(lhs, facts);
        let rhs_range = int_range_for_value(rhs, facts);
        if let (Some(lhs_range), Some(rhs_range)) = (lhs_range, rhs_range)
            && let Some(intersection) = lhs_range.intersect(rhs_range)
        {
            constrain_register(facts, lhs, intersection);
            constrain_register(facts, rhs, intersection);
        }
    }
}

fn refine_register_vs_const(
    facts: &mut Facts,
    kind: CompareKind,
    register: &IRValue,
    constant: i64,
    want_compare: bool,
    constant_on_left: bool,
) {
    let range = match (kind, want_compare, constant_on_left) {
        (CompareKind::Eq, true, false) | (CompareKind::Eq, true, true) => IntRange {
            min: constant,
            max: constant,
        },
        (CompareKind::Lt, true, false) => IntRange {
            min: i64::MIN,
            max: constant.saturating_sub(1),
        },
        (CompareKind::Lt, false, false) => IntRange {
            min: constant,
            max: i64::MAX,
        },
        (CompareKind::Lt, true, true) => IntRange {
            min: constant.saturating_add(1),
            max: i64::MAX,
        },
        (CompareKind::Lt, false, true) => IntRange {
            min: i64::MIN,
            max: constant,
        },
        (CompareKind::Lte, true, false) => IntRange {
            min: i64::MIN,
            max: constant,
        },
        (CompareKind::Lte, false, false) => IntRange {
            min: constant.saturating_add(1),
            max: i64::MAX,
        },
        (CompareKind::Lte, true, true) => IntRange {
            min: constant,
            max: i64::MAX,
        },
        (CompareKind::Lte, false, true) => IntRange {
            min: i64::MIN,
            max: constant.saturating_sub(1),
        },
        (CompareKind::Eq, false, _) => return,
    };

    constrain_register(facts, register, range);
}

fn constrain_register(facts: &mut Facts, value: &IRValue, constraint: IntRange) {
    let IRValue::Register(_, _) = value else {
        return;
    };

    let constrained = match facts.get(value).and_then(int_range_for_fact) {
        Some(existing) => existing.intersect(constraint),
        None => Some(constraint),
    };

    let Some(constrained) = constrained else {
        return;
    };
    facts.insert(value.clone(), range_to_fact(constrained));
}

fn fact_for_value(value: &IRValue, current: &Facts) -> Option<Fact> {
    match value {
        IRValue::Register(_, _) => current.get(value).cloned(),
        IRValue::Constant(value) => Some(Fact::Constant(*value)),
    }
}

fn exact_constant(fact: &Fact) -> Option<JSValue> {
    match fact {
        Fact::Constant(value) => Some(*value),
        Fact::Range(range) if range.min == range.max => Some(int_to_value(range.min)),
        Fact::Range(_) => None,
    }
}

fn exact_int_for_value(value: &IRValue, current: &Facts) -> Option<i64> {
    int_range_for_value(value, current)
        .and_then(|range| (range.min == range.max).then_some(range.min))
}

fn int_range_for_value(value: &IRValue, current: &Facts) -> Option<IntRange> {
    let fact = fact_for_value(value, current)?;
    int_range_for_fact(&fact)
}

fn int_range_for_fact(fact: &Fact) -> Option<IntRange> {
    match fact {
        Fact::Constant(value) => exact_int_from_value(*value).map(|value| IntRange {
            min: value,
            max: value,
        }),
        Fact::Range(range) => Some(*range),
    }
}

fn exact_int_from_value(value: JSValue) -> Option<i64> {
    if let Some(value) = value.as_i32() {
        return Some(value as i64);
    }

    let number = value.as_f64()?;
    if !number.is_finite() || number.fract() != 0.0 {
        return None;
    }

    Some(number as i64)
}

fn range_to_fact(range: IntRange) -> Fact {
    if range.min == range.max {
        Fact::Constant(int_to_value(range.min))
    } else {
        Fact::Range(range)
    }
}

fn int_to_value(value: i64) -> JSValue {
    if (i32::MIN as i64..=i32::MAX as i64).contains(&value) {
        make_int32(value as i32)
    } else {
        make_number(value as f64)
    }
}

fn join_maps(left: &Facts, right: &Facts) -> Facts {
    let mut joined = Facts::new();

    for (key, left_fact) in left {
        let Some(right_fact) = right.get(key) else {
            continue;
        };
        let Some(fact) = join_fact(left_fact, right_fact) else {
            continue;
        };
        joined.insert(key.clone(), fact);
    }

    joined
}

fn join_fact(left: &Fact, right: &Fact) -> Option<Fact> {
    if left == right {
        return Some(left.clone());
    }

    let left_range = int_range_for_fact(left)?;
    let right_range = int_range_for_fact(right)?;
    Some(Fact::Range(IntRange {
        min: left_range.min.min(right_range.min),
        max: left_range.max.max(right_range.max),
    }))
}

fn widen_maps(previous: &Facts, next: &Facts) -> Facts {
    let mut widened = Facts::new();

    for (key, next_fact) in next {
        let fact = match previous.get(key) {
            Some(previous_fact) => widen_fact(previous_fact, next_fact),
            None => Some(next_fact.clone()),
        };

        if let Some(fact) = fact {
            widened.insert(key.clone(), fact);
        }
    }

    widened
}

fn widen_fact(previous: &Fact, next: &Fact) -> Option<Fact> {
    if previous == next {
        return Some(previous.clone());
    }

    let previous_range = int_range_for_fact(previous)?;
    let next_range = int_range_for_fact(next)?;
    Some(Fact::Range(previous_range.widen(next_range)))
}

impl IntRange {
    fn widen(self, next: IntRange) -> Self {
        Self {
            min: if next.min < self.min {
                i64::MIN
            } else {
                next.min
            },
            max: if next.max > self.max {
                i64::MAX
            } else {
                next.max
            },
        }
    }

    fn intersect(self, other: IntRange) -> Option<Self> {
        let min = self.min.max(other.min);
        let max = self.max.min(other.max);
        (min <= max).then_some(Self { min, max })
    }
}

fn recompute_cfg_links(ir: &mut IRFunction) {
    let mut predecessors = vec![Vec::new(); ir.blocks.len()];

    for block in &mut ir.blocks {
        block.successors = terminator_successors(&block.terminator);
    }

    for block_id in 0..ir.blocks.len() {
        for &successor in &ir.blocks[block_id].successors {
            if successor < predecessors.len() && !predecessors[successor].contains(&block_id) {
                predecessors[successor].push(block_id);
            }
        }
    }

    let mut exit_blocks = Vec::new();
    for block_id in 0..ir.blocks.len() {
        ir.blocks[block_id].predecessors = predecessors[block_id].clone();
        if is_exit_terminator(&ir.blocks[block_id].terminator) {
            exit_blocks.push(block_id);
        }
    }
    ir.exit_blocks = exit_blocks;
}

fn terminator_successors(terminator: &IRTerminator) -> Vec<BlockId> {
    match terminator {
        IRTerminator::None
        | IRTerminator::Return { .. }
        | IRTerminator::Throw { .. }
        | IRTerminator::TailCall { .. }
        | IRTerminator::CallReturn { .. } => Vec::new(),
        IRTerminator::Jump { target } => vec![*target],
        IRTerminator::Branch {
            target,
            fallthrough,
            ..
        } => {
            let mut successors = vec![*target];
            if *fallthrough != *target {
                successors.push(*fallthrough);
            }
            successors
        }
        IRTerminator::Switch {
            cases,
            default_target,
            ..
        } => {
            let mut successors = vec![*default_target];
            for (_, target) in cases {
                if !successors.contains(target) {
                    successors.push(*target);
                }
            }
            successors
        }
        IRTerminator::Try {
            handler,
            fallthrough,
        } => {
            let mut successors = vec![*handler];
            if *fallthrough != *handler {
                successors.push(*fallthrough);
            }
            successors
        }
        IRTerminator::ConditionalReturn { fallthrough, .. } => vec![*fallthrough],
    }
}

fn is_exit_terminator(terminator: &IRTerminator) -> bool {
    matches!(
        terminator,
        IRTerminator::Return { .. }
            | IRTerminator::Throw { .. }
            | IRTerminator::TailCall { .. }
            | IRTerminator::CallReturn { .. }
    )
}

fn push_unique_successor(edges: &mut Vec<(BlockId, Facts)>, block: BlockId, facts: Facts) {
    if edges.iter().any(|(candidate, _)| *candidate == block) {
        return;
    }
    edges.push((block, facts));
}
