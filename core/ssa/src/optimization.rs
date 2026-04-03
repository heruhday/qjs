use std::collections::{BTreeSet, HashMap};

use cfg::{ACC_REG, BlockId, CompareKind, DecodedInst};
use codegen::Opcode;
use value::{JSValue, make_false, make_true};

use crate::ir::{
    BytecodeLoweringError, IRBinaryOp, IRBlock, IRCondition, IRFunction, IRInst, IRTerminator,
    IRUnaryOp, IRValue,
};
use crate::passes::{
    CfgSimplification, ConstantFolding, CopyPropagation, DeadCodeElimination, GlobalValueNumbering,
    LoopInvariantCodeMotion, Pass, SparseConditionalConstantPropagation, ValueRangePropagation,
};

pub fn simplify_branches(ir: &mut IRFunction) -> bool {
    CfgSimplification.run(ir)
}

pub fn constant_fold(ir: &mut IRFunction) -> bool {
    ConstantFolding.run(ir)
}

pub fn copy_propagation(ir: &mut IRFunction) -> bool {
    CopyPropagation.run(ir)
}

pub fn eliminate_dead_code(ir: &mut IRFunction) -> bool {
    DeadCodeElimination.run(ir)
}

pub fn loop_invariant_code_motion(ir: &mut IRFunction) -> bool {
    LoopInvariantCodeMotion.run(ir)
}

pub fn fold_temporary_checks(ir: &mut IRFunction) -> bool {
    let mut changed = false;

    for block in &mut ir.blocks {
        let mut known_values = HashMap::<IRValue, JSValue>::new();

        for inst in &mut block.instructions {
            match inst {
                IRInst::Phi { .. } | IRInst::Unary { .. } | IRInst::Binary { .. } => {
                    let replacement = match inst {
                        IRInst::Unary {
                            dst,
                            op: IRUnaryOp::IsUndef,
                            operand,
                        } => constant_for_value(operand, &known_values).map(|value| {
                            IRInst::LoadConst {
                                dst: dst.clone(),
                                value: if value.is_undefined() {
                                    make_true()
                                } else {
                                    make_false()
                                },
                            }
                        }),
                        IRInst::Unary {
                            dst,
                            op: IRUnaryOp::IsNull,
                            operand,
                        } => constant_for_value(operand, &known_values).map(|value| {
                            IRInst::LoadConst {
                                dst: dst.clone(),
                                value: if value.is_null() {
                                    make_true()
                                } else {
                                    make_false()
                                },
                            }
                        }),
                        _ => None,
                    };

                    if let Some(replacement) = replacement {
                        let IRInst::LoadConst { dst, value } = &replacement else {
                            unreachable!("temporary check folding only emits constants");
                        };
                        known_values.insert(dst.clone(), *value);
                        *inst = replacement;
                        changed = true;
                    } else if let Some(dst) = defined_value(inst) {
                        known_values.remove(&dst);
                    }
                }
                IRInst::Mov { dst, src } => {
                    if let Some(value) = constant_for_value(src, &known_values) {
                        known_values.insert(dst.clone(), value);
                    } else {
                        known_values.remove(dst);
                    }
                }
                IRInst::LoadConst { dst, value } => {
                    known_values.insert(dst.clone(), *value);
                }
                IRInst::Bytecode { defs, .. } => {
                    for def in defs {
                        known_values.remove(def);
                    }
                }
                IRInst::Nop => {}
            }
        }
    }

    changed
}

pub fn optimize_basic_peephole(ir: &mut IRFunction) -> bool {
    let mut changed = false;

    for block in &mut ir.blocks {
        for _ in 0..3 {
            let mut local_changed = false;

            for inst in &mut block.instructions {
                if let IRInst::Mov { dst, src } = inst
                    && dst == src
                {
                    *inst = IRInst::Nop;
                    local_changed = true;
                }
            }

            for index in 1..block.instructions.len() {
                let previous = block.instructions[index - 1].clone();
                let current = block.instructions[index].clone();
                let (prev_replacement, curr_replacement) = match (&previous, &current) {
                    (
                        IRInst::LoadConst {
                            dst: previous_dst,
                            value,
                        },
                        IRInst::Mov { dst, src },
                    ) if src == previous_dst => (
                        None,
                        Some(IRInst::LoadConst { dst: dst.clone(), value: *value }),
                    ),
                    (
                        IRInst::Mov {
                            dst: previous_dst,
                            src: previous_src,
                        },
                        IRInst::Mov { dst, src },
                    ) if src == previous_dst => (
                        None,
                        Some(IRInst::Mov {
                            dst: dst.clone(),
                            src: previous_src.clone(),
                        }),
                    ),
                    (
                        IRInst::Binary {
                            dst: previous_dst,
                            op,
                            lhs,
                            rhs,
                        },
                        IRInst::Mov { dst, src },
                    ) if src == previous_dst => (
                        Some(IRInst::Nop),
                        Some(IRInst::Binary {
                            dst: dst.clone(),
                            op: *op,
                            lhs: lhs.clone(),
                            rhs: rhs.clone(),
                        }),
                    ),
                    (
                        IRInst::Unary {
                            dst: previous_dst,
                            op,
                            operand,
                        },
                        IRInst::Mov { dst, src },
                    ) if src == previous_dst => (
                        Some(IRInst::Nop),
                        Some(IRInst::Unary {
                            dst: dst.clone(),
                            op: *op,
                            operand: operand.clone(),
                        }),
                    ),
                    _ => (None, None),
                };

                if let Some(replacement) = curr_replacement
                    && block.instructions[index] != replacement
                {
                    if let Some(prev_repl) = prev_replacement {
                        block.instructions[index - 1] = prev_repl;
                    }
                    block.instructions[index] = replacement;
                    local_changed = true;
                }
            }

            if local_changed {
                changed = true;
            } else {
                break;
            }
        }

        let original_len = block.instructions.len();
        block
            .instructions
            .retain(|inst| !matches!(inst, IRInst::Nop));
        if block.instructions.len() != original_len {
            changed = true;
        }
    }

    changed
}

pub fn optimize_superinstructions(ir: &mut IRFunction) -> bool {
    let mut changed = false;

    for block in &mut ir.blocks {
        // First, try to fuse binary operations into branches
        changed |= fuse_compare_into_branch(block);
        
        // Try to fuse binary operations into special bytecode instructions
        changed |= fuse_binary_into_bytecode(block);
        
        // Try to fuse bytecode calls into returns
        changed |= fuse_call_into_return(block);
        
        // Then lift terminal values
        loop {
            let Some(last) = block.instructions.last().cloned() else {
                break;
            };

            let Some((from, to)) = lift_terminal_value(last) else {
                break;
            };

            if !replace_terminator_uses(&mut block.terminator, &from, &to) {
                break;
            }

            block.instructions.pop();
            changed = true;
        }
    }

    changed
}

/// Fuse Binary/Unary operations into special bytecode instructions
fn fuse_binary_into_bytecode(block: &mut IRBlock) -> bool {
    let mut changed = false;
    
    let mut i = 0;
    while i < block.instructions.len() {
        match &block.instructions[i] {
            IRInst::Binary {
                dst,
                op: IRBinaryOp::Eq,
                lhs,
                rhs,
            } => {
                // Check if only used in Return
                if let IRTerminator::Return {
                    value: Some(ret_value),
                } = &block.terminator
                {
                    if ret_value == dst {
                        // Create LoadCmpEq bytecode instruction
                        let inst = DecodedInst {
                            opcode: Opcode::LoadCmpEq,
                            a: 0,
                            b: 0,
                            c: 0,
                            sbx: 0,
                            bx: 0,
                            raw: 0,
                            pc: 0,
                        };
                        block.instructions[i] = IRInst::Bytecode {
                            inst,
                            uses: vec![lhs.clone(), rhs.clone()],
                            defs: vec![dst.clone()],
                        };
                        changed = true;
                    }
                }
            }
            IRInst::Unary {
                dst,
                op: IRUnaryOp::Inc,
                operand,
            } => {
                // Check if only used in Return
                if let IRTerminator::Return {
                    value: Some(ret_value),
                } = &block.terminator
                {
                    if ret_value == dst {
                        // Create LoadInc bytecode instruction
                        let inst = DecodedInst {
                            opcode: Opcode::LoadInc,
                            a: 0,
                            b: 0,
                            c: 0,
                            sbx: 0,
                            bx: 0,
                            raw: 0,
                            pc: 0,
                        };
                        block.instructions[i] = IRInst::Bytecode {
                            inst,
                            uses: vec![operand.clone()],
                            defs: vec![dst.clone()],
                        };
                        changed = true;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    
    changed
}

/// Fuse Call bytecode instruction directly into CallReturn terminator
fn fuse_call_into_return(block: &mut IRBlock) -> bool {
    let mut changed = false;
    
    if block.instructions.is_empty() {
        return false;
    }
    
    // Check if last instruction is a Call bytecode operation
    if let Some(last_idx) = block.instructions.len().checked_sub(1) {
        if let IRInst::Bytecode { inst, uses, defs } = &block.instructions[last_idx] {
            // Check if it returns in the terminator
            if let IRTerminator::Return {
                value: Some(ret_value),
            } = &block.terminator
            {
                // Check if the only def is used in the return
                if !defs.is_empty() && defs.len() < 2 && ret_value == &defs[0] {
                    // Check if it's a Call opcode (b carries the argc argument)
                    if inst.opcode == Opcode::Call && !uses.is_empty() && uses.len() > 1 {
                        let callee = uses[1].clone();
                        let argc = inst.b;
                        block.terminator = IRTerminator::CallReturn { callee, argc };
                        block.instructions.pop();
                        changed = true;
                    }
                }
            }
        }
    }
    
    changed
}

/// Fuse binary compare operations directly into branch conditions
fn fuse_compare_into_branch(block: &mut IRBlock) -> bool {
    let mut changed = false;
    
    if block.instructions.is_empty() {
        return false;
    }
    
    // Check if last instruction is a compare operation
    if let Some(last_idx) = block.instructions.len().checked_sub(1) {
        if let IRInst::Binary {
            dst,
            op: binary_op,
            lhs,
            rhs,
        } = &block.instructions[last_idx]
        {
            // Check if the compare result is only used in the terminator
            if let IRTerminator::Branch {
                condition: IRCondition::Truthy {
                    value: cond_value,
                    negate,
                },
                target,
                fallthrough,
            } = &block.terminator
            {
                if cond_value == dst {
                    // Fuse the compare into the branch
                    let compare_kind = match binary_op {
                        IRBinaryOp::Lt => Some(CompareKind::Lt),
                        IRBinaryOp::Lte => Some(CompareKind::Lte),
                        IRBinaryOp::Eq => Some(CompareKind::Neq), // Eq becomes Neq with negate
                        _ => None,
                    };
                    
                    if let Some(kind) = compare_kind {
                        block.terminator = IRTerminator::Branch {
                            condition: IRCondition::Compare {
                                kind,
                                lhs: lhs.clone(),
                                rhs: rhs.clone(),
                                negate: *negate,
                            },
                            target: *target,
                            fallthrough: *fallthrough,
                        };
                        block.instructions.pop();
                        changed = true;
                    }
                }
            }
        }
    }
    
    changed
}

pub fn coalesce_registers(ir: &mut IRFunction) -> bool {
    let mut changed = false;
    changed |= copy_propagation(ir);
    changed |= optimize_superinstructions(ir);
    changed |= optimize_basic_peephole(ir);
    changed |= eliminate_dead_code(ir);
    changed
}

pub fn reuse_registers_linear_scan(ir: &mut IRFunction) -> bool {
    if contains_opaque_bytecode(ir) {
        return false;
    }

    let intervals = live_intervals(ir);
    if intervals.is_empty() {
        return false;
    }

    let Some(mapping) = allocate_registers(&intervals) else {
        return false;
    };

    let mut changed = false;

    for block in &mut ir.blocks {
        for inst in &mut block.instructions {
            changed |= rewrite_instruction_registers(inst, &mapping);
        }
        changed |= rewrite_terminator_registers(&mut block.terminator, &mapping);
    }

    changed
}

pub fn run_fixed_point_round(ir: &mut IRFunction) -> bool {
    let mut changed = false;
    changed |= simplify_branches(ir);
    changed |= constant_fold(ir);
    changed |= fold_temporary_checks(ir);
    changed |= copy_propagation(ir);
    changed |= optimize_basic_peephole(ir);
    changed |= eliminate_dead_code(ir);
    changed |= eliminate_dead_code(ir);
    changed
}

fn run_cleanup_round(ir: &mut IRFunction) -> bool {
    let mut changed = false;
    changed |= simplify_branches(ir);
    changed |= constant_fold(ir);
    changed |= fold_temporary_checks(ir);
    changed |= copy_propagation(ir);
    changed |= optimize_superinstructions(ir);
    changed |= optimize_basic_peephole(ir);
    changed |= eliminate_dead_code(ir);
    changed |= eliminate_dead_code(ir);
    changed
}

pub fn run_until_stable<F>(ir: &mut IRFunction, max_rounds: usize, mut round: F) -> bool
where
    F: FnMut(&mut IRFunction) -> bool,
{
    let mut changed = false;

    for _ in 0..max_rounds.max(1) {
        let round_changed = round(ir);
        changed |= round_changed;
        if !round_changed {
            break;
        }
    }

    changed
}

pub fn optimize_ir(ir: &mut IRFunction) -> bool {
    let mut changed = false;

    changed |= run_until_stable(ir, 8, run_fixed_point_round);
    changed |= loop_invariant_code_motion(ir);
    changed |= GlobalValueNumbering.run(ir);
    changed |= SparseConditionalConstantPropagation.run(ir);
    changed |= ValueRangePropagation.run(ir);
    changed |= loop_invariant_code_motion(ir);
    changed |= run_until_stable(ir, 4, run_cleanup_round);

    changed
}

pub fn optimize_bytecode(ir: &mut IRFunction) -> bool {
    let mut changed = optimize_ir(ir);
    changed |= reuse_registers_linear_scan(ir);
    changed |= optimize_superinstructions(ir);
    changed |= optimize_basic_peephole(ir);
    changed |= simplify_branches(ir);
    changed |= eliminate_dead_code(ir);
    changed
}

pub fn optimize_to_bytecode(
    ir: &IRFunction,
) -> Result<(Vec<u32>, Vec<JSValue>), BytecodeLoweringError> {
    let mut optimized_ir = ir.clone();
    optimize_bytecode(&mut optimized_ir);
    optimized_ir.into_bytecodes()
}

#[derive(Debug, Clone)]
struct LiveInterval {
    value: IRValue,
    start: usize,
    end: usize,
}

fn contains_opaque_bytecode(ir: &IRFunction) -> bool {
    ir.blocks.iter().any(|block| {
        block
            .instructions
            .iter()
            .any(|inst| matches!(inst, IRInst::Bytecode { .. }))
    })
}

fn lift_terminal_value(inst: IRInst) -> Option<(IRValue, IRValue)> {
    match inst {
        IRInst::Mov { dst, src } => Some((dst, src)),
        IRInst::LoadConst { dst, value } => Some((dst, IRValue::Constant(value))),
        _ => None,
    }
}

fn replace_terminator_uses(terminator: &mut IRTerminator, from: &IRValue, to: &IRValue) -> bool {
    let mut changed = false;

    match terminator {
        IRTerminator::Branch { condition, .. } => {
            changed |= replace_condition_uses(condition, from, to);
        }
        IRTerminator::Switch { key, .. }
        | IRTerminator::Throw { value: key }
        | IRTerminator::TailCall { callee: key, .. }
        | IRTerminator::CallReturn { callee: key, .. } => {
            changed |= replace_value_use(key, from, to);
        }
        IRTerminator::ConditionalReturn {
            condition, value, ..
        } => {
            changed |= replace_condition_uses(condition, from, to);
            changed |= replace_value_use(value, from, to);
        }
        IRTerminator::Return { value } => {
            if let Some(value) = value {
                changed |= replace_value_use(value, from, to);
            }
        }
        IRTerminator::Jump { .. } | IRTerminator::Try { .. } | IRTerminator::None => {}
    }

    changed
}

fn replace_condition_uses(condition: &mut IRCondition, from: &IRValue, to: &IRValue) -> bool {
    match condition {
        IRCondition::Truthy { value, .. } => replace_value_use(value, from, to),
        IRCondition::Compare { lhs, rhs, .. } => {
            let mut changed = false;
            changed |= replace_value_use(lhs, from, to);
            changed |= replace_value_use(rhs, from, to);
            changed
        }
    }
}

fn replace_value_use(value: &mut IRValue, from: &IRValue, to: &IRValue) -> bool {
    if value != from {
        return false;
    }

    *value = to.clone();
    true
}

fn live_intervals(ir: &IRFunction) -> Vec<LiveInterval> {
    let mut definitions = HashMap::<IRValue, usize>::new();
    let mut uses = HashMap::<IRValue, usize>::new();
    let mut terminator_positions = HashMap::<BlockId, usize>::new();
    let mut position = 0usize;

    for block in &ir.blocks {
        for inst in &block.instructions {
            if let Some(dst) = defined_value(inst) {
                definitions.entry(dst).or_insert(position);
            }

            collect_instruction_uses(inst, position + 1, &mut uses);
            position += 2;
        }

        terminator_positions.insert(block.id, position);
        collect_terminator_uses(&block.terminator, position + 1, &mut uses);
        position += 2;
    }

    for block in &ir.blocks {
        for inst in &block.instructions {
            let IRInst::Phi { incoming, .. } = inst else {
                continue;
            };

            for (pred, value) in incoming {
                let Some(position) = terminator_positions.get(pred).copied() else {
                    continue;
                };
                record_use(value, position + 1, &mut uses);
            }
        }
    }

    let mut intervals = definitions
        .into_iter()
        .filter_map(|(value, start)| match value {
            IRValue::Register(reg, version) => Some(LiveInterval {
                end: uses.get(&value).copied().unwrap_or(start),
                start,
                value: IRValue::Register(reg, version),
            }),
            IRValue::Constant(_) => None,
        })
        .collect::<Vec<_>>();

    intervals.sort_by_key(|interval| match &interval.value {
        IRValue::Register(reg, version) => (interval.start, interval.end, *reg, *version),
        IRValue::Constant(_) => (interval.start, interval.end, 0, 0),
    });
    intervals
}

fn allocate_registers(intervals: &[LiveInterval]) -> Option<HashMap<IRValue, u8>> {
    let mut mapping = HashMap::<IRValue, u8>::new();
    let mut free = (0..ACC_REG).collect::<BTreeSet<_>>();
    let mut active = Vec::<(usize, u8)>::new();

    for interval in intervals {
        active.retain(|(end, reg)| {
            if *end < interval.start {
                free.insert(*reg);
                false
            } else {
                true
            }
        });

        let IRValue::Register(original_reg, _) = interval.value else {
            continue;
        };

        if original_reg == ACC_REG {
            mapping.insert(interval.value.clone(), ACC_REG);
            continue;
        }

        let Some(physical) = free.pop_first() else {
            return None;
        };
        mapping.insert(interval.value.clone(), physical);
        active.push((interval.end, physical));
    }

    Some(mapping)
}

fn rewrite_instruction_registers(inst: &mut IRInst, mapping: &HashMap<IRValue, u8>) -> bool {
    let mut changed = false;

    match inst {
        IRInst::Phi { dst, incoming } => {
            changed |= rewrite_value_register(dst, mapping);
            for (_, value) in incoming {
                changed |= rewrite_value_register(value, mapping);
            }
        }
        IRInst::Mov { dst, src } => {
            changed |= rewrite_value_register(dst, mapping);
            changed |= rewrite_value_register(src, mapping);
        }
        IRInst::LoadConst { dst, .. } => {
            changed |= rewrite_value_register(dst, mapping);
        }
        IRInst::Unary { dst, operand, .. } => {
            changed |= rewrite_value_register(dst, mapping);
            changed |= rewrite_value_register(operand, mapping);
        }
        IRInst::Binary { dst, lhs, rhs, .. } => {
            changed |= rewrite_value_register(dst, mapping);
            changed |= rewrite_value_register(lhs, mapping);
            changed |= rewrite_value_register(rhs, mapping);
        }
        IRInst::Bytecode { .. } | IRInst::Nop => {}
    }

    changed
}

fn rewrite_terminator_registers(
    terminator: &mut IRTerminator,
    mapping: &HashMap<IRValue, u8>,
) -> bool {
    let mut changed = false;

    match terminator {
        IRTerminator::Branch { condition, .. } => {
            changed |= rewrite_condition_registers(condition, mapping);
        }
        IRTerminator::Switch { key, .. }
        | IRTerminator::Throw { value: key }
        | IRTerminator::TailCall { callee: key, .. }
        | IRTerminator::CallReturn { callee: key, .. } => {
            changed |= rewrite_value_register(key, mapping);
        }
        IRTerminator::ConditionalReturn {
            condition, value, ..
        } => {
            changed |= rewrite_condition_registers(condition, mapping);
            changed |= rewrite_value_register(value, mapping);
        }
        IRTerminator::Return { value } => {
            if let Some(value) = value {
                changed |= rewrite_value_register(value, mapping);
            }
        }
        IRTerminator::Jump { .. } | IRTerminator::Try { .. } | IRTerminator::None => {}
    }

    changed
}

fn rewrite_condition_registers(
    condition: &mut IRCondition,
    mapping: &HashMap<IRValue, u8>,
) -> bool {
    match condition {
        IRCondition::Truthy { value, .. } => rewrite_value_register(value, mapping),
        IRCondition::Compare { lhs, rhs, .. } => {
            let mut changed = false;
            changed |= rewrite_value_register(lhs, mapping);
            changed |= rewrite_value_register(rhs, mapping);
            changed
        }
    }
}

fn rewrite_value_register(value: &mut IRValue, mapping: &HashMap<IRValue, u8>) -> bool {
    let IRValue::Register(reg, version) = value else {
        return false;
    };

    let key = IRValue::Register(*reg, *version);
    let Some(new_reg) = mapping.get(&key).copied() else {
        return false;
    };
    if new_reg == *reg {
        return false;
    }

    *reg = new_reg;
    true
}

fn collect_instruction_uses(inst: &IRInst, position: usize, uses: &mut HashMap<IRValue, usize>) {
    match inst {
        IRInst::Phi { .. } | IRInst::LoadConst { .. } | IRInst::Nop => {}
        IRInst::Mov { src, .. } => {
            record_use(src, position, uses);
        }
        IRInst::Unary { operand, .. } => {
            record_use(operand, position, uses);
        }
        IRInst::Binary { lhs, rhs, .. } => {
            record_use(lhs, position, uses);
            record_use(rhs, position, uses);
        }
        IRInst::Bytecode { uses: values, .. } => {
            for value in values {
                record_use(value, position, uses);
            }
        }
    }
}

fn collect_terminator_uses(
    terminator: &IRTerminator,
    position: usize,
    uses: &mut HashMap<IRValue, usize>,
) {
    match terminator {
        IRTerminator::Branch { condition, .. }
        | IRTerminator::ConditionalReturn { condition, .. } => {
            collect_condition_uses(condition, position, uses);
        }
        IRTerminator::Switch { key, .. }
        | IRTerminator::Throw { value: key }
        | IRTerminator::TailCall { callee: key, .. }
        | IRTerminator::CallReturn { callee: key, .. } => {
            record_use(key, position, uses);
        }
        IRTerminator::Return { value } => {
            if let Some(value) = value {
                record_use(value, position, uses);
            }
        }
        IRTerminator::Jump { .. } | IRTerminator::Try { .. } | IRTerminator::None => {}
    }
}

fn collect_condition_uses(
    condition: &IRCondition,
    position: usize,
    uses: &mut HashMap<IRValue, usize>,
) {
    match condition {
        IRCondition::Truthy { value, .. } => {
            record_use(value, position, uses);
        }
        IRCondition::Compare { lhs, rhs, .. } => {
            record_use(lhs, position, uses);
            record_use(rhs, position, uses);
        }
    }
}

fn record_use(value: &IRValue, position: usize, uses: &mut HashMap<IRValue, usize>) {
    if !matches!(value, IRValue::Register(_, _)) {
        return;
    }

    uses.entry(value.clone())
        .and_modify(|existing| *existing = (*existing).max(position))
        .or_insert(position);
}

fn constant_for_value(
    value: &IRValue,
    known_values: &HashMap<IRValue, JSValue>,
) -> Option<JSValue> {
    match value {
        IRValue::Constant(value) => Some(*value),
        IRValue::Register(_, _) => known_values.get(value).copied(),
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

/// Optimization tier for selective optimization application
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptTier {
    /// Tier 0: Peephole optimizations without register reuse (cold code)
    Tier0,
    /// Tier 1: All SSA optimizations including copy propagation, constant folding, etc.
    Tier1,
    /// Tier 2: All SSA optimizations + register coalescing (hot code)
    Tier2,
}

/// Multi-tiered optimizer that applies different optimization levels
pub struct Optimizer {
    tier: OptTier,
}

impl Optimizer {
    pub fn new(tier: OptTier) -> Self {
        Optimizer { tier }
    }

    pub fn optimize(&self, ir: &mut IRFunction) -> bool {
        match self.tier {
            OptTier::Tier0 => optimize_tier0(ir),
            OptTier::Tier1 => optimize_ir(ir),
            OptTier::Tier2 => optimize_tier2(ir),
        }
    }
}

/// Tier 0 optimization: Basic peephole passes without register reuse
/// Used for cold code paths
pub fn optimize_tier0(ir: &mut IRFunction) -> bool {
    let mut changed = false;

    // Apply basic peephole optimizations without register reuse
    changed |= simplify_branches(ir);
    changed |= constant_fold(ir);
    changed |= copy_propagation(ir);
    changed |= eliminate_dead_code(ir);
    changed |= optimize_basic_peephole(ir);
    changed |= optimize_superinstructions(ir);
    changed |= fold_temporary_checks(ir);

    changed
}

/// Tier 2 optimization: All optimizations including register reuse
/// Used for hot code paths (higher optimization cost justified by frequency)
pub fn optimize_tier2(ir: &mut IRFunction) -> bool {
    // Run all optimizations including register reuse
    optimize_ir(ir) | reuse_registers_linear_scan(ir)
}

/// Mixed bytecode and IR optimization - implemented in bytecode_fallback module
/// This is a stub to maintain the public API (actual implementation in bytecode_fallback)
#[allow(dead_code)]
fn peephole_remove_redundant_movs(bytecode: &[u32]) -> Vec<u32> {
    // This is just a helper for potential bytecode optimizations
    // The actual optimization is implemented in bytecode_fallback::optimize_mixed_bytecode
    bytecode.to_vec()
}
