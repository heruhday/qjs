use std::collections::{BTreeSet, HashMap, HashSet};

use cfg::{ACC_REG, BlockId, CompareKind, decode_word};
use codegen::Opcode;
use value::{JSValue, make_false, make_true};

use crate::ir::{
    BytecodeLoweringError, IRBinaryOp, IRBlock, IRCondition, IRFunction, IRInst, IRTerminator,
    IRUnaryOp, IRValue,
};
use crate::optimize_mixed_bytecode;
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
    let numeric = infer_numeric_values(ir);
    let mut changed = false;

    for block in &mut ir.blocks {
        changed |= optimize_peephole_block(block, &numeric, PeepholeMode::basic());
    }

    changed
}

pub fn optimize_superinstructions(ir: &mut IRFunction) -> bool {
    let numeric = infer_numeric_values(ir);
    let mut changed = false;

    for block in &mut ir.blocks {
        changed |= optimize_peephole_block(block, &numeric, PeepholeMode::superinstructions());
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptTier {
    Tier0,
    Tier1,
    Tier2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Optimizer {
    pub tier: OptTier,
}

impl Optimizer {
    pub const fn new(tier: OptTier) -> Self {
        Self { tier }
    }

    pub fn optimize(&self, ir: &mut IRFunction) -> bool {
        match self.tier {
            OptTier::Tier0 => optimize_tier0(ir),
            OptTier::Tier1 => optimize_tier1(ir),
            OptTier::Tier2 => optimize_tier2(ir),
        }
    }

    pub fn optimize_to_bytecode(
        &self,
        ir: &IRFunction,
    ) -> Result<(Vec<u32>, Vec<JSValue>), BytecodeLoweringError> {
        optimize_to_bytecode_with_tier(ir, self.tier)
    }
}

impl Default for Optimizer {
    fn default() -> Self {
        Self::new(OptTier::Tier1)
    }
}

pub fn run_basic_round(ir: &mut IRFunction) -> bool {
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

pub fn run_fixed_point_round(ir: &mut IRFunction) -> bool {
    run_basic_round(ir)
}

pub fn run_full_round(ir: &mut IRFunction) -> bool {
    let mut changed = false;
    changed |= run_basic_round(ir);
    changed |= loop_invariant_code_motion(ir);
    changed |= GlobalValueNumbering.run(ir);
    changed |= SparseConditionalConstantPropagation.run(ir);
    changed |= ValueRangePropagation.run(ir);
    changed |= optimize_basic_peephole(ir);
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

pub fn optimize_tier0(ir: &mut IRFunction) -> bool {
    let mut changed = false;
    changed |= run_until_stable(ir, 2, run_basic_round);
    changed |= optimize_superinstructions(ir);
    changed |= eliminate_dead_code(ir);
    changed
}

pub fn optimize_tier1(ir: &mut IRFunction) -> bool {
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

pub fn optimize_tier2(ir: &mut IRFunction) -> bool {
    let mut changed = optimize_tier1(ir);
    changed |= reuse_registers_linear_scan(ir);
    changed |= optimize_superinstructions(ir);
    changed |= optimize_basic_peephole(ir);
    changed |= simplify_branches(ir);
    changed |= eliminate_dead_code(ir);
    changed
}

pub fn optimize_ir(ir: &mut IRFunction) -> bool {
    optimize_tier1(ir)
}

pub fn optimize_bytecode(ir: &mut IRFunction) -> bool {
    optimize_tier2(ir)
}

pub fn optimize_to_bytecode(
    ir: &IRFunction,
) -> Result<(Vec<u32>, Vec<JSValue>), BytecodeLoweringError> {
    optimize_to_bytecode_with_tier(ir, OptTier::Tier2)
}

pub fn optimize_to_bytecode_with_tier(
    ir: &IRFunction,
    tier: OptTier,
) -> Result<(Vec<u32>, Vec<JSValue>), BytecodeLoweringError> {
    let mut optimized_ir = ir.clone();
    Optimizer::new(tier).optimize(&mut optimized_ir);
    optimized_ir
        .into_bytecodes()
        .map(|(bytecode, constants)| optimize_mixed_bytecode(bytecode, constants))
}

#[derive(Debug, Clone)]
struct LiveInterval {
    value: IRValue,
    start: usize,
    end: usize,
}

#[derive(Clone, Copy)]
struct PeepholeMode {
    structural: bool,
    superinstructions: bool,
}

impl PeepholeMode {
    const fn basic() -> Self {
        Self {
            structural: true,
            superinstructions: false,
        }
    }

    const fn superinstructions() -> Self {
        Self {
            structural: false,
            superinstructions: true,
        }
    }
}

fn contains_opaque_bytecode(ir: &IRFunction) -> bool {
    ir.blocks.iter().any(|block| {
        block
            .instructions
            .iter()
            .any(|inst| matches!(inst, IRInst::Bytecode { .. }))
    })
}

fn optimize_peephole_block(
    block: &mut IRBlock,
    numeric: &HashSet<IRValue>,
    mode: PeepholeMode,
) -> bool {
    let mut changed = false;

    loop {
        let mut local_changed = false;

        if mode.structural {
            local_changed |= apply_structural_peepholes(block);
        }

        if mode.superinstructions {
            local_changed |= apply_superinstruction_peepholes(block, numeric);
        }

        if !local_changed {
            break;
        }

        changed = true;
    }

    changed
}

fn apply_structural_peepholes(block: &mut IRBlock) -> bool {
    let mut changed = false;

    for inst in &mut block.instructions {
        if let IRInst::Mov { dst, src } = inst
            && dst == src
        {
            *inst = IRInst::Nop;
            changed = true;
        }
    }

    for index in 1..block.instructions.len() {
        let previous = block.instructions[index - 1].clone();
        let current = block.instructions[index].clone();

        if let IRInst::Mov { dst, src } = &current
            && let Some(previous_dst) = defined_value(&previous)
            && *src == previous_dst
            && can_sink_destination(&previous)
            && !value_used_after(block, index, &previous_dst)
            && rewrite_destination(&mut block.instructions[index - 1], dst)
        {
            block.instructions[index] = IRInst::Nop;
            changed = true;
            continue;
        }

        if let Some(replacement) = structural_replacement(previous, current)
            && block.instructions[index] != replacement
        {
            block.instructions[index] = replacement;
            changed = true;
        }
    }

    let original_len = block.instructions.len();
    block
        .instructions
        .retain(|inst| !matches!(inst, IRInst::Nop));
    changed || block.instructions.len() != original_len
}

fn structural_replacement(previous: IRInst, current: IRInst) -> Option<IRInst> {
    match (previous, current) {
        (
            IRInst::LoadConst {
                dst: previous_dst,
                value,
            },
            IRInst::Mov { dst, src },
        ) if src == previous_dst => Some(IRInst::LoadConst { dst, value }),
        (
            IRInst::Mov {
                dst: previous_dst,
                src: previous_src,
            },
            IRInst::Mov { dst, src },
        ) if src == previous_dst => Some(IRInst::Mov {
            dst,
            src: previous_src,
        }),
        _ => None,
    }
}

fn apply_superinstruction_peepholes(block: &mut IRBlock, numeric: &HashSet<IRValue>) -> bool {
    let mut changed = false;

    loop {
        if try_lift_terminal_value(block)
            || try_lift_terminal_compare(block)
            || try_fuse_terminal_call_return(block)
        {
            changed = true;
            continue;
        }
        break;
    }

    let mut index = 0;
    while index < block.instructions.len() {
        if try_fuse_instruction_superinstruction(block, index, numeric) {
            changed = true;
            continue;
        }
        index += 1;
    }

    changed
}

fn try_fuse_instruction_superinstruction(
    block: &mut IRBlock,
    index: usize,
    numeric: &HashSet<IRValue>,
) -> bool {
    let Some(inst) = block.instructions.get(index).cloned() else {
        return false;
    };

    let replacement = match inst {
        IRInst::Unary { dst, op, operand } => match op {
            IRUnaryOp::Inc => {
                let (Some(dst_reg), Some(src_reg)) =
                    (register_value(&dst), register_value(&operand))
                else {
                    return false;
                };
                Some(make_bytecode_inst(
                    Opcode::LoadInc,
                    dst_reg,
                    src_reg,
                    0,
                    vec![operand],
                    vec![dst],
                ))
            }
            IRUnaryOp::Dec => {
                let (Some(dst_reg), Some(src_reg)) =
                    (register_value(&dst), register_value(&operand))
                else {
                    return false;
                };
                Some(make_bytecode_inst(
                    Opcode::LoadDec,
                    dst_reg,
                    src_reg,
                    0,
                    vec![operand],
                    vec![dst],
                ))
            }
            _ => None,
        },
        IRInst::Binary { dst, op, lhs, rhs } => {
            let (Some(dst_reg), Some(lhs_reg), Some(rhs_reg)) = (
                register_value(&dst),
                register_value(&lhs),
                register_value(&rhs),
            ) else {
                return false;
            };

            let both_numeric = value_is_numeric(&lhs, numeric) && value_is_numeric(&rhs, numeric);
            match op {
                IRBinaryOp::Add if !both_numeric => Some(make_bytecode_inst(
                    Opcode::LoadAdd,
                    dst_reg,
                    lhs_reg,
                    rhs_reg,
                    vec![lhs, rhs],
                    vec![dst],
                )),
                IRBinaryOp::Sub if !both_numeric => Some(make_bytecode_inst(
                    Opcode::LoadSub,
                    dst_reg,
                    lhs_reg,
                    rhs_reg,
                    vec![lhs, rhs],
                    vec![dst],
                )),
                IRBinaryOp::Mul if !both_numeric => Some(make_bytecode_inst(
                    Opcode::LoadMul,
                    dst_reg,
                    lhs_reg,
                    rhs_reg,
                    vec![lhs, rhs],
                    vec![dst],
                )),
                IRBinaryOp::Eq => Some(make_bytecode_inst(
                    Opcode::LoadCmpEq,
                    dst_reg,
                    lhs_reg,
                    rhs_reg,
                    vec![lhs, rhs],
                    vec![dst],
                )),
                IRBinaryOp::Lt if !both_numeric => Some(make_bytecode_inst(
                    Opcode::LoadCmpLt,
                    dst_reg,
                    lhs_reg,
                    rhs_reg,
                    vec![lhs, rhs],
                    vec![dst],
                )),
                _ => None,
            }
        }
        _ => None,
    };

    let Some(replacement) = replacement else {
        return false;
    };

    if block.instructions[index] == replacement {
        return false;
    }

    block.instructions[index] = replacement;
    true
}

fn register_value(value: &IRValue) -> Option<u8> {
    match value {
        IRValue::Register(reg, _) => Some(*reg),
        IRValue::Constant(_) => None,
    }
}

fn encode_raw(opcode: Opcode, a: u8, b: u8, c: u8) -> u32 {
    ((c as u32) << 24) | ((b as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}

fn make_bytecode_inst(
    opcode: Opcode,
    a: u8,
    b: u8,
    c: u8,
    uses: Vec<IRValue>,
    defs: Vec<IRValue>,
) -> IRInst {
    IRInst::Bytecode {
        inst: decode_word(0, encode_raw(opcode, a, b, c)),
        uses,
        defs,
    }
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

fn try_lift_terminal_value(block: &mut IRBlock) -> bool {
    let Some(last) = block.instructions.last().cloned() else {
        return false;
    };

    let Some((from, to)) = lift_terminal_value(last) else {
        return false;
    };

    if !replace_terminator_uses(&mut block.terminator, &from, &to) {
        return false;
    }

    block.instructions.pop();
    true
}

fn try_lift_terminal_compare(block: &mut IRBlock) -> bool {
    let Some(IRInst::Binary { dst, op, lhs, rhs }) = block.instructions.last().cloned() else {
        return false;
    };
    let Some(kind) = compare_kind_for_binary(op) else {
        return false;
    };

    let condition = match &mut block.terminator {
        IRTerminator::Branch { condition, .. }
        | IRTerminator::ConditionalReturn { condition, .. } => condition,
        _ => return false,
    };

    let negate = match condition {
        IRCondition::Truthy { value, negate } if *value == dst => *negate,
        _ => return false,
    };

    *condition = IRCondition::Compare {
        kind,
        lhs,
        rhs,
        negate,
    };
    block.instructions.pop();
    true
}

fn try_fuse_terminal_call_return(block: &mut IRBlock) -> bool {
    let returned = match &block.terminator {
        IRTerminator::Return { value: Some(value) } => value.clone(),
        _ => return false,
    };

    let Some(IRInst::Bytecode { inst, uses, defs }) = block.instructions.last().cloned() else {
        return false;
    };
    if inst.opcode != codegen::Opcode::Call || !defs.iter().any(|value| *value == returned) {
        return false;
    }

    let Some(callee) = uses
        .iter()
        .find(|value| matches!(value, IRValue::Register(reg, _) if *reg == inst.a))
        .cloned()
    else {
        return false;
    };

    block.instructions.pop();
    block.terminator = IRTerminator::CallReturn {
        callee,
        argc: inst.b,
    };
    true
}

fn compare_kind_for_binary(op: IRBinaryOp) -> Option<CompareKind> {
    match op {
        IRBinaryOp::Eq => Some(CompareKind::Eq),
        IRBinaryOp::Lt => Some(CompareKind::Lt),
        IRBinaryOp::Lte => Some(CompareKind::Lte),
        _ => None,
    }
}

fn can_sink_destination(inst: &IRInst) -> bool {
    matches!(
        inst,
        IRInst::Mov { .. }
            | IRInst::LoadConst { .. }
            | IRInst::Unary { .. }
            | IRInst::Binary { .. }
    )
}

fn rewrite_destination(inst: &mut IRInst, dst: &IRValue) -> bool {
    match inst {
        IRInst::Mov { dst: current, .. }
        | IRInst::LoadConst { dst: current, .. }
        | IRInst::Unary { dst: current, .. }
        | IRInst::Binary { dst: current, .. } => {
            if *current == *dst {
                false
            } else {
                *current = dst.clone();
                true
            }
        }
        IRInst::Phi { .. } | IRInst::Bytecode { .. } | IRInst::Nop => false,
    }
}

fn value_used_after(block: &IRBlock, index: usize, value: &IRValue) -> bool {
    block.instructions[index + 1..]
        .iter()
        .any(|inst| instruction_uses_value(inst, value))
        || terminator_uses_value(&block.terminator, value)
}

fn instruction_uses_value(inst: &IRInst, value: &IRValue) -> bool {
    match inst {
        IRInst::Phi { incoming, .. } => incoming.iter().any(|(_, incoming)| incoming == value),
        IRInst::Mov { src, .. } => src == value,
        IRInst::LoadConst { .. } | IRInst::Nop => false,
        IRInst::Unary { operand, .. } => operand == value,
        IRInst::Binary { lhs, rhs, .. } => lhs == value || rhs == value,
        IRInst::Bytecode { uses, .. } => uses.iter().any(|used| used == value),
    }
}

fn terminator_uses_value(terminator: &IRTerminator, value: &IRValue) -> bool {
    match terminator {
        IRTerminator::Branch { condition, .. }
        | IRTerminator::ConditionalReturn { condition, .. } => {
            condition_uses_value(condition, value)
        }
        IRTerminator::Switch { key, .. }
        | IRTerminator::Throw { value: key }
        | IRTerminator::TailCall { callee: key, .. }
        | IRTerminator::CallReturn { callee: key, .. } => key == value,
        IRTerminator::Return {
            value: Some(returned),
        } => returned == value,
        IRTerminator::Jump { .. }
        | IRTerminator::Try { .. }
        | IRTerminator::Return { value: None }
        | IRTerminator::None => false,
    }
}

fn infer_numeric_values(ir: &IRFunction) -> HashSet<IRValue> {
    let mut numeric = HashSet::new();

    loop {
        let mut changed = false;

        for block in &ir.blocks {
            for inst in &block.instructions {
                let Some(dst) = defined_value(inst) else {
                    continue;
                };

                let is_numeric = match inst {
                    IRInst::Phi { incoming, .. } => {
                        !incoming.is_empty()
                            && incoming
                                .iter()
                                .all(|(_, value)| value_is_numeric(value, &numeric))
                    }
                    IRInst::Mov { src, .. } => value_is_numeric(src, &numeric),
                    IRInst::LoadConst { value, .. } => value::to_f64(*value).is_some(),
                    IRInst::Unary { op, .. } => matches!(
                        op,
                        IRUnaryOp::ToNum
                            | IRUnaryOp::Neg
                            | IRUnaryOp::Inc
                            | IRUnaryOp::Dec
                            | IRUnaryOp::BitNot
                    ),
                    IRInst::Binary { op, lhs, rhs, .. } => match op {
                        IRBinaryOp::Add => {
                            value_is_numeric(lhs, &numeric) && value_is_numeric(rhs, &numeric)
                        }
                        IRBinaryOp::Sub
                        | IRBinaryOp::Mul
                        | IRBinaryOp::Div
                        | IRBinaryOp::Mod
                        | IRBinaryOp::Pow
                        | IRBinaryOp::BitAnd
                        | IRBinaryOp::BitOr
                        | IRBinaryOp::BitXor
                        | IRBinaryOp::Shl
                        | IRBinaryOp::Shr
                        | IRBinaryOp::Ushr => true,
                        IRBinaryOp::Eq
                        | IRBinaryOp::Lt
                        | IRBinaryOp::Lte
                        | IRBinaryOp::StrictEq
                        | IRBinaryOp::StrictNeq
                        | IRBinaryOp::LogicalAnd
                        | IRBinaryOp::LogicalOr
                        | IRBinaryOp::NullishCoalesce
                        | IRBinaryOp::In
                        | IRBinaryOp::Instanceof
                        | IRBinaryOp::AddStr => false,
                    },
                    IRInst::Bytecode { inst, .. } => matches!(
                        inst.opcode,
                        Opcode::LoadI
                            | Opcode::AddI32
                            | Opcode::AddF64
                            | Opcode::SubI32
                            | Opcode::SubF64
                            | Opcode::MulI32
                            | Opcode::MulF64
                            | Opcode::AddI32Fast
                            | Opcode::AddF64Fast
                            | Opcode::SubI32Fast
                            | Opcode::MulI32Fast
                            | Opcode::LoadInc
                            | Opcode::LoadDec
                    ),
                    IRInst::Nop => false,
                };

                if is_numeric && numeric.insert(dst) {
                    changed = true;
                }
            }
        }

        if !changed {
            break;
        }
    }

    numeric
}

fn value_is_numeric(value: &IRValue, numeric: &HashSet<IRValue>) -> bool {
    match value {
        IRValue::Constant(value) => value::to_f64(*value).is_some(),
        IRValue::Register(_, _) => numeric.contains(value),
    }
}

fn condition_uses_value(condition: &IRCondition, value: &IRValue) -> bool {
    match condition {
        IRCondition::Truthy {
            value: condition_value,
            ..
        } => condition_value == value,
        IRCondition::Compare { lhs, rhs, .. } => lhs == value || rhs == value,
    }
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
