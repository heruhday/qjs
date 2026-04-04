use std::collections::{BTreeSet, HashMap};

use cfg::{ACC_REG, DecodedInst};
use codegen::Opcode;
use value::{
    JSValue, is_null, is_undefined, make_false, make_int32, make_null, make_number, make_true,
    to_f64,
};

use crate::semantics::instruction_operands;

const REG_COUNT: usize = u8::MAX as usize + 1;

#[derive(Clone, Debug)]
struct Instruction {
    opcode: Opcode,
    a: u8,
    b: u8,
    c: u8,
    bx: u16,
    sbx: i16,
    target: Option<usize>,
    removed: bool,
}

impl Instruction {
    fn decode(pc: usize, raw: u32) -> Self {
        let opcode = Opcode::from((raw & 0xFF) as u8);
        let a = ((raw >> 8) & 0xFF) as u8;
        let b = ((raw >> 16) & 0xFF) as u8;
        let c = ((raw >> 24) & 0xFF) as u8;
        let bx = ((raw >> 16) & 0xFFFF) as u16;
        let sbx = bx as i16;
        let target = decode_branch_target(opcode, pc, a, c, sbx);

        Self {
            opcode,
            a,
            b,
            c,
            bx,
            sbx,
            target,
            removed: false,
        }
    }

    fn decoded(&self, pc: usize) -> DecodedInst {
        DecodedInst {
            raw: 0,
            pc,
            opcode: self.opcode,
            a: self.a,
            b: self.b,
            c: self.c,
            bx: self.bx,
            sbx: self.sbx,
        }
    }

    fn encode(&self, pc: usize, boundary_map: &[usize]) -> u32 {
        match self.opcode {
            Opcode::Jmp
            | Opcode::JmpTrue
            | Opcode::JmpFalse
            | Opcode::LoopIncJmp
            | Opcode::Try
            | Opcode::IncJmpFalseLoop
            | Opcode::IncAccJmp
            | Opcode::TestJmpTrue => {
                let target = self.target.unwrap_or(pc + 1).min(boundary_map.len() - 1);
                let offset = boundary_map[target] as isize - (boundary_map[pc] as isize + 1);
                let sbx = i16::try_from(offset).expect("optimized jump offset must fit in i16");
                (((sbx as u16) as u32) << 16) | ((self.a as u32) << 8) | self.opcode.as_u8() as u32
            }
            Opcode::JmpEq
            | Opcode::JmpNeq
            | Opcode::JmpLt
            | Opcode::JmpLtF64
            | Opcode::JmpLte
            | Opcode::JmpLteF64
            | Opcode::JmpLteFalse
            | Opcode::JmpLteFalseF64
            | Opcode::JmpI32Fast
            | Opcode::CmpJmp
            | Opcode::LoadCmpEqJfalse
            | Opcode::LoadCmpLtJfalse => {
                let target = self.target.unwrap_or(pc + 1).min(boundary_map.len() - 1);
                let offset = boundary_map[target] as isize - (boundary_map[pc] as isize + 1);
                let offset =
                    i8::try_from(offset).expect("optimized conditional jump offset must fit in i8");
                ((offset as u8 as u32) << 24)
                    | ((self.b as u32) << 16)
                    | ((self.a as u32) << 8)
                    | self.opcode.as_u8() as u32
            }
            Opcode::EqJmpTrue | Opcode::LtJmp | Opcode::EqJmpFalse | Opcode::LteJmpLoop => {
                let target = self.target.unwrap_or(pc + 1).min(boundary_map.len() - 1);
                let offset = boundary_map[target] as isize - (boundary_map[pc] as isize + 1);
                let offset =
                    i8::try_from(offset).expect("optimized conditional jump offset must fit in i8");
                ((self.c as u32) << 24)
                    | ((self.b as u32) << 16)
                    | ((offset as u8 as u32) << 8)
                    | self.opcode.as_u8() as u32
            }
            Opcode::LoadI => {
                (((self.sbx as u16) as u32) << 16)
                    | ((self.a as u32) << 8)
                    | self.opcode.as_u8() as u32
            }
            Opcode::LoadK
            | Opcode::LoadGlobalIc
            | Opcode::SetGlobalIc
            | Opcode::NewFunc
            | Opcode::GetGlobal
            | Opcode::SetGlobal
            | Opcode::ResolveScope
            | Opcode::LoadName
            | Opcode::StoreName
            | Opcode::TypeofName
            | Opcode::LoadKAddAcc
            | Opcode::LoadKMulAcc
            | Opcode::LoadKSubAcc
            | Opcode::CallMethod1
            | Opcode::CallMethod2
            | Opcode::Enter => {
                ((self.bx as u32) << 16) | ((self.a as u32) << 8) | self.opcode.as_u8() as u32
            }
            _ => {
                ((self.c as u32) << 24)
                    | ((self.b as u32) << 16)
                    | ((self.a as u32) << 8)
                    | self.opcode.as_u8() as u32
            }
        }
    }

    fn new_mov(dst: u8, src: u8) -> Self {
        Self {
            opcode: Opcode::Mov,
            a: dst,
            b: src,
            c: 0,
            bx: src as u16,
            sbx: src as i16,
            target: None,
            removed: false,
        }
    }

    fn new_load_acc(src: u8) -> Self {
        Self {
            opcode: Opcode::LoadAcc,
            a: src,
            b: 0,
            c: 0,
            bx: src as u16,
            sbx: src as i16,
            target: None,
            removed: false,
        }
    }

    fn new_load_i(dst: u8, value: i16) -> Self {
        Self {
            opcode: Opcode::LoadI,
            a: dst,
            b: 0,
            c: 0,
            bx: value as u16,
            sbx: value,
            target: None,
            removed: false,
        }
    }

    fn new_load_true(dst: u8) -> Self {
        Self {
            opcode: Opcode::LoadTrue,
            a: dst,
            b: 0,
            c: 0,
            bx: 0,
            sbx: 0,
            target: None,
            removed: false,
        }
    }

    fn new_load_false(dst: u8) -> Self {
        Self {
            opcode: Opcode::LoadFalse,
            a: dst,
            b: 0,
            c: 0,
            bx: 0,
            sbx: 0,
            target: None,
            removed: false,
        }
    }

    fn new_load_null(dst: u8) -> Self {
        Self {
            opcode: Opcode::LoadNull,
            a: dst,
            b: 0,
            c: 0,
            bx: 0,
            sbx: 0,
            target: None,
            removed: false,
        }
    }

    fn new_load_k(dst: u8, index: u16) -> Self {
        Self {
            opcode: Opcode::LoadK,
            a: dst,
            b: 0,
            c: 0,
            bx: index,
            sbx: index as i16,
            target: None,
            removed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RegisterValueKey {
    Immediate(i16),
    Constant(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KnownValueKind {
    Unknown,
    Undefined,
    Null,
    NonNullish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackedValue {
    Register(u8),
    Literal(JSValue),
}

impl TrackedValue {
    fn uses_register(self, reg: u8) -> bool {
        matches!(self, Self::Register(source) if source == reg)
    }
}

#[derive(Clone, Debug)]
struct InstructionSemantics {
    uses: Vec<u8>,
    defs: Vec<u8>,
    successors: Vec<usize>,
    pinned: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LiveInterval {
    reg: u8,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug)]
struct LivenessAnalysis {
    intervals: [Option<LiveInterval>; REG_COUNT],
    pinned: [bool; REG_COUNT],
}

pub fn optimize_mixed_bytecode(
    bytecode: Vec<u32>,
    mut constants: Vec<JSValue>,
) -> (Vec<u32>, Vec<JSValue>) {
    if bytecode.is_empty() {
        return (bytecode, constants);
    }

    let original_bytecode = bytecode.clone();
    let original_constants = constants.clone();
    let mut insts = decode_program(&bytecode);
    let mut changed = false;

    for _ in 0..8 {
        let mut round_changed = false;
        round_changed |= run_fold_temporary_checks(&mut insts, &mut constants);
        round_changed |= run_block_pass(&mut insts, &constants, |insts, start, end, _| {
            coalesce_registers_block(insts, start, end)
        });
        {
            let leaders = collect_block_leaders(&insts, &constants);
            for (block_index, &start) in leaders.iter().enumerate() {
                if start >= insts.len() {
                    continue;
                }
                let end = leaders
                    .get(block_index + 1)
                    .copied()
                    .unwrap_or(insts.len())
                    .min(insts.len());
                round_changed |= copy_propagation_block(&mut insts, &mut constants, start, end);
            }
        }
        round_changed |= run_block_pass(&mut insts, &constants, |insts, start, end, _| {
            optimize_basic_peephole_block(insts, start, end)
        });
        round_changed |= run_block_pass(&mut insts, &constants, eliminate_dead_defs);
        round_changed |= thread_jumps(&mut insts);
        changed |= round_changed;
        if !round_changed {
            break;
        }
    }

    changed |= reuse_registers_linear_scan(&mut insts, &constants);
    changed |= run_block_pass(&mut insts, &constants, eliminate_dead_defs);
    changed |= run_block_pass(&mut insts, &constants, |insts, start, end, _| {
        optimize_basic_peephole_block(insts, start, end)
    });
    changed |= thread_jumps(&mut insts);

    if !changed {
        return (original_bytecode, original_constants);
    }

    encode_program(&insts, constants)
}

fn decode_branch_target(opcode: Opcode, pc: usize, a: u8, c: u8, sbx: i16) -> Option<usize> {
    let next = pc as isize + 1;
    let target = match opcode {
        Opcode::Jmp
        | Opcode::JmpTrue
        | Opcode::JmpFalse
        | Opcode::LoopIncJmp
        | Opcode::Try
        | Opcode::IncJmpFalseLoop
        | Opcode::IncAccJmp
        | Opcode::TestJmpTrue => next + sbx as isize,
        Opcode::JmpEq
        | Opcode::JmpNeq
        | Opcode::JmpLt
        | Opcode::JmpLtF64
        | Opcode::JmpLte
        | Opcode::JmpLteF64
        | Opcode::JmpLteFalse
        | Opcode::JmpLteFalseF64
        | Opcode::JmpI32Fast
        | Opcode::CmpJmp
        | Opcode::LoadCmpEqJfalse
        | Opcode::LoadCmpLtJfalse => next + c as i8 as isize,
        Opcode::EqJmpTrue | Opcode::LtJmp | Opcode::EqJmpFalse | Opcode::LteJmpLoop => {
            next + a as i8 as isize
        }
        _ => return None,
    };

    Some(target.max(0) as usize)
}

fn decode_program(bytecode: &[u32]) -> Vec<Instruction> {
    bytecode
        .iter()
        .enumerate()
        .map(|(pc, &raw)| Instruction::decode(pc, raw))
        .collect()
}

fn build_boundary_map(insts: &[Instruction]) -> Vec<usize> {
    let mut boundary = vec![0; insts.len() + 1];
    let mut next = insts.iter().filter(|inst| !inst.removed).count();
    boundary[insts.len()] = next;

    for index in (0..insts.len()).rev() {
        if !insts[index].removed {
            next -= 1;
        }
        boundary[index] = next;
    }

    boundary
}

fn rewrite_switch_tables(insts: &[Instruction], constants: &mut [JSValue], boundary: &[usize]) {
    for (pc, inst) in insts.iter().enumerate() {
        if inst.removed || inst.opcode != Opcode::Switch {
            continue;
        }

        let table_index = inst.b as usize;
        let Some(case_count) = constants.get(table_index).and_then(|value| to_f64(*value)) else {
            continue;
        };
        let case_count = case_count as usize;
        let base = boundary[pc] as isize + 1;

        if let Some(default_slot) = constants.get_mut(table_index + 1)
            && let Some(old_offset) = to_f64(*default_slot)
        {
            let old_target = ((pc + 1) as isize + old_offset as i16 as isize).max(0) as usize;
            let new_target = boundary[old_target.min(insts.len())] as isize;
            *default_slot = make_number((new_target - base) as f64);
        }

        for case_index in 0..case_count {
            let offset_index = table_index + 2 + case_index * 2 + 1;
            if let Some(slot) = constants.get_mut(offset_index)
                && let Some(old_offset) = to_f64(*slot)
            {
                let old_target = ((pc + 1) as isize + old_offset as i16 as isize).max(0) as usize;
                let new_target = boundary[old_target.min(insts.len())] as isize;
                *slot = make_number((new_target - base) as f64);
            }
        }
    }
}

fn encode_program(insts: &[Instruction], mut constants: Vec<JSValue>) -> (Vec<u32>, Vec<JSValue>) {
    let boundary = build_boundary_map(insts);
    let bytecode = insts
        .iter()
        .enumerate()
        .filter(|(_, inst)| !inst.removed)
        .map(|(pc, inst)| inst.encode(pc, &boundary))
        .collect::<Vec<_>>();
    rewrite_switch_tables(insts, &mut constants, &boundary);
    (bytecode, constants)
}

fn switch_targets(pc: usize, table_index: usize, constants: &[JSValue]) -> Vec<usize> {
    let Some(case_count) = constants.get(table_index).and_then(|value| to_f64(*value)) else {
        return Vec::new();
    };
    let case_count = case_count as usize;
    let mut targets = Vec::with_capacity(case_count + 1);

    if let Some(default_offset) = constants
        .get(table_index + 1)
        .and_then(|value| to_f64(*value))
    {
        targets.push(((pc + 1) as isize + default_offset as i16 as isize).max(0) as usize);
    }

    for case_index in 0..case_count {
        let offset_index = table_index + 2 + case_index * 2 + 1;
        if let Some(offset) = constants.get(offset_index).and_then(|value| to_f64(*value)) {
            targets.push(((pc + 1) as isize + offset as i16 as isize).max(0) as usize);
        }
    }

    targets
}

fn is_terminator(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Jmp | Opcode::Ret | Opcode::RetU | Opcode::RetReg | Opcode::Throw | Opcode::CallRet
    )
}

fn collect_block_leaders(insts: &[Instruction], constants: &[JSValue]) -> Vec<usize> {
    let mut leaders = BTreeSet::new();
    leaders.insert(0);

    for (pc, inst) in insts.iter().enumerate() {
        if let Some(target) = inst.target {
            leaders.insert(target.min(insts.len()));
            if pc + 1 < insts.len() {
                leaders.insert(pc + 1);
            }
        }

        if inst.opcode == Opcode::Switch {
            leaders.extend(
                switch_targets(pc, inst.b as usize, constants)
                    .into_iter()
                    .map(|target| target.min(insts.len())),
            );
            if pc + 1 < insts.len() {
                leaders.insert(pc + 1);
            }
        }

        if is_terminator(inst.opcode) && pc + 1 < insts.len() {
            leaders.insert(pc + 1);
        }
    }

    leaders.into_iter().collect()
}

fn run_block_pass<F>(insts: &mut [Instruction], constants: &[JSValue], mut pass: F) -> bool
where
    F: FnMut(&mut [Instruction], usize, usize, bool) -> bool,
{
    let leaders = collect_block_leaders(insts, constants);
    let mut changed = false;

    for (block_index, &start) in leaders.iter().enumerate() {
        if start >= insts.len() {
            continue;
        }
        let end = leaders
            .get(block_index + 1)
            .copied()
            .unwrap_or(insts.len())
            .min(insts.len());
        let terminal = (start..end)
            .rev()
            .find(|&index| !insts[index].removed)
            .is_some_and(|index| {
                matches!(
                    insts[index].opcode,
                    Opcode::Ret | Opcode::RetU | Opcode::RetReg | Opcode::Throw | Opcode::CallRet
                )
            });
        changed |= pass(insts, start, end, terminal);
    }

    changed
}

fn run_fold_temporary_checks(insts: &mut [Instruction], constants: &mut Vec<JSValue>) -> bool {
    let leaders = collect_block_leaders(insts, constants);
    let mut changed = false;
    for (block_index, &start) in leaders.iter().enumerate() {
        if start >= insts.len() {
            continue;
        }
        let end = leaders
            .get(block_index + 1)
            .copied()
            .unwrap_or(insts.len())
            .min(insts.len());
        changed |= fold_temporary_checks_block(insts, constants, start, end);
    }
    changed
}

fn mark_call_bundle_live(live: &mut [bool; REG_COUNT], base: u8, argc: u8) {
    let start = usize::from(base);
    let end = (start + usize::from(argc)).min(usize::from(ACC_REG) - 1);
    for reg in start..=end {
        live[reg] = true;
    }
}

fn eliminate_dead_defs(
    insts: &mut [Instruction],
    start: usize,
    end: usize,
    terminal: bool,
) -> bool {
    let mut changed = false;
    let mut live = [false; REG_COUNT];

    if !terminal {
        live.fill(true);
    }

    for index in (start..end).rev() {
        if insts[index].removed {
            continue;
        }

        match insts[index].opcode {
            Opcode::Mov => {
                let dst = insts[index].a as usize;
                let src = insts[index].b as usize;
                if !live[dst] {
                    insts[index].removed = true;
                    changed = true;
                    continue;
                }
                live[dst] = false;
                live[src] = true;
            }
            Opcode::LoadI | Opcode::LoadK => {
                let dst = insts[index].a as usize;
                if !live[dst] {
                    insts[index].removed = true;
                    changed = true;
                    continue;
                }
                live[dst] = false;
            }
            Opcode::LoadGlobalIc
            | Opcode::GetGlobal
            | Opcode::GetUpval
            | Opcode::GetScope
            | Opcode::ResolveScope
            | Opcode::NewArr
            | Opcode::NewFunc
            | Opcode::NewThis
            | Opcode::LoadClosure
            | Opcode::TypeofName
            | Opcode::CreateEnv
            | Opcode::LoadArg
            | Opcode::LoadRestArgs => {
                live[insts[index].a as usize] = false;
            }
            Opcode::NewClass
            | Opcode::Typeof
            | Opcode::ToNum
            | Opcode::ToStr
            | Opcode::IsUndef
            | Opcode::IsNull
            | Opcode::DeleteProp
            | Opcode::HasProp
            | Opcode::Keys => {
                live[insts[index].a as usize] = false;
                live[insts[index].b as usize] = true;
            }
            Opcode::ForIn => {
                live[insts[index].a as usize] = false;
                live[ACC_REG as usize] = false;
                live[insts[index].b as usize] = true;
            }
            Opcode::IteratorNext => {
                live[ACC_REG as usize] = false;
                live[insts[index].a as usize] = true;
            }
            Opcode::Add
            | Opcode::Eq
            | Opcode::Lt
            | Opcode::Lte
            | Opcode::StrictEq
            | Opcode::StrictNeq
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor
            | Opcode::Shl
            | Opcode::Shr
            | Opcode::Ushr
            | Opcode::Pow
            | Opcode::LogicalAnd
            | Opcode::LogicalOr
            | Opcode::NullishCoalesce
            | Opcode::In
            | Opcode::Instanceof
            | Opcode::AddStr
            | Opcode::EqI32Fast
            | Opcode::LtI32Fast => {
                live[ACC_REG as usize] = false;
                live[insts[index].b as usize] = true;
                live[insts[index].c as usize] = true;
            }
            Opcode::AddAcc
            | Opcode::SubAcc
            | Opcode::MulAcc
            | Opcode::DivAcc
            | Opcode::AddStrAcc
            | Opcode::Neg
            | Opcode::Inc
            | Opcode::Dec
            | Opcode::ToPrimitive
            | Opcode::BitNot => {
                live[ACC_REG as usize] = true;
                live[insts[index].b as usize] = true;
            }
            Opcode::AddAccImm8
            | Opcode::SubAccImm8
            | Opcode::MulAccImm8
            | Opcode::DivAccImm8
            | Opcode::IncAcc
            | Opcode::LoadKAddAcc
            | Opcode::LoadKMulAcc
            | Opcode::LoadKSubAcc
            | Opcode::IncAccJmp => {
                live[ACC_REG as usize] = true;
            }
            Opcode::LoadThis
            | Opcode::Load0
            | Opcode::Load1
            | Opcode::LoadNull
            | Opcode::LoadTrue
            | Opcode::LoadFalse => {
                live[ACC_REG as usize] = false;
                if matches!(
                    insts[index].opcode,
                    Opcode::LoadTrue | Opcode::LoadFalse | Opcode::LoadNull
                ) {
                    live[insts[index].a as usize] = false;
                } else if insts[index].opcode == Opcode::LoadThis {
                    live[0] = true;
                }
            }
            Opcode::LoadAcc => {
                live[ACC_REG as usize] = false;
                live[insts[index].a as usize] = true;
            }
            Opcode::LoadName => {
                let dst = insts[index].a as usize;
                let acc = ACC_REG as usize;
                if !live[dst] && !live[acc] {
                    insts[index].removed = true;
                    changed = true;
                    continue;
                }
                live[dst] = false;
                live[acc] = false;
            }
            Opcode::SetGlobalIc
            | Opcode::SetGlobal
            | Opcode::SetUpval
            | Opcode::SetScope
            | Opcode::StoreName
            | Opcode::InitName => {
                live[insts[index].a as usize] = true;
            }
            Opcode::NewObj => {
                let dst = insts[index].a as usize;
                if !live[dst] {
                    insts[index].removed = true;
                    changed = true;
                    continue;
                }
                live[dst] = false;
            }
            Opcode::GetProp | Opcode::GetPropIc | Opcode::GetSuper | Opcode::GetLengthIc => {
                let dst = insts[index].a as usize;
                if !live[dst] {
                    insts[index].removed = true;
                    changed = true;
                    continue;
                }
                live[dst] = false;
                live[insts[index].b as usize] = true;
            }
            Opcode::SetProp | Opcode::SetPropIc | Opcode::SetSuper => {
                live[ACC_REG as usize] = false;
                live[insts[index].a as usize] = true;
                live[insts[index].b as usize] = true;
            }
            Opcode::JmpTrue | Opcode::JmpFalse | Opcode::TestJmpTrue => {
                live[insts[index].a as usize] = true;
            }
            Opcode::JmpEq
            | Opcode::JmpNeq
            | Opcode::JmpLt
            | Opcode::JmpLtF64
            | Opcode::JmpLte
            | Opcode::JmpLteF64
            | Opcode::JmpLteFalse
            | Opcode::JmpLteFalseF64
            | Opcode::JmpI32Fast
            | Opcode::CmpJmp
            | Opcode::LoadCmpEqJfalse
            | Opcode::LoadCmpLtJfalse => {
                live[insts[index].a as usize] = true;
                live[insts[index].b as usize] = true;
            }
            Opcode::LoopIncJmp => {
                live[insts[index].a as usize] = true;
                live[ACC_REG as usize] = true;
            }
            Opcode::EqJmpTrue | Opcode::LtJmp | Opcode::EqJmpFalse | Opcode::LteJmpLoop => {
                live[insts[index].b as usize] = true;
                live[insts[index].c as usize] = true;
            }
            Opcode::Ret => {
                live[ACC_REG as usize] = true;
            }
            Opcode::RetReg => {
                live[insts[index].a as usize] = true;
            }
            Opcode::Call
            | Opcode::TailCall
            | Opcode::Construct
            | Opcode::CallIc
            | Opcode::CallIcSuper
            | Opcode::CallMono => {
                live[ACC_REG as usize] = false;
                live[0] = true;
                mark_call_bundle_live(&mut live, insts[index].a, insts[index].b);
            }
            Opcode::CallRet => {
                live[0] = true;
                mark_call_bundle_live(&mut live, insts[index].a, insts[index].b);
            }
            Opcode::Call0 => {
                live[ACC_REG as usize] = false;
                live[0] = true;
                live[insts[index].a as usize] = true;
            }
            Opcode::Call1 => {
                live[ACC_REG as usize] = false;
                live[0] = true;
                live[insts[index].a as usize] = true;
                live[insts[index].b as usize] = true;
            }
            Opcode::Call2 => {
                live[ACC_REG as usize] = false;
                live[0] = true;
                live[insts[index].a as usize] = true;
                live[insts[index].b as usize] = true;
                live[insts[index].c as usize] = true;
            }
            Opcode::Call3 => {
                live[ACC_REG as usize] = false;
                live[0] = true;
                live[insts[index].a as usize] = true;
                live[insts[index].b as usize] = true;
                live[insts[index].c as usize] = true;
            }
            Opcode::CallMethod1 => {
                live[ACC_REG as usize] = false;
                live[0] = true;
                mark_call_bundle_live(&mut live, insts[index].a, 1);
            }
            Opcode::CallMethod2 => {
                live[ACC_REG as usize] = false;
                live[0] = true;
                mark_call_bundle_live(&mut live, insts[index].a, 2);
            }
            Opcode::CallThis => {
                live[ACC_REG as usize] = false;
                live[insts[index].b as usize] = true;
                mark_call_bundle_live(&mut live, insts[index].a, insts[index].c);
            }
            Opcode::CallThisVar => {
                live[ACC_REG as usize] = false;
                live[insts[index].a as usize] = true;
                live[insts[index].b as usize] = true;
                live[insts[index].c as usize] = true;
            }
            Opcode::CallMethodIc | Opcode::CallMethod2Ic => {
                live[ACC_REG as usize] = false;
                live[insts[index].a as usize] = true;
            }
            Opcode::Call1Add => {
                live[ACC_REG as usize] = true;
                live[0] = true;
                live[insts[index].a as usize] = true;
                live[insts[index].b as usize] = true;
            }
            Opcode::Call2Add => {
                live[ACC_REG as usize] = true;
                live[0] = true;
                live[insts[index].a as usize] = true;
                live[insts[index].b as usize] = true;
                live[insts[index].c as usize] = true;
            }
            Opcode::RetU | Opcode::Jmp | Opcode::Switch | Opcode::LoopHint => {}
            _ => {
                live.fill(true);
            }
        }
    }

    changed
}

fn invalidate_value_key(
    available: &mut HashMap<RegisterValueKey, u8>,
    values: &mut [Option<RegisterValueKey>; REG_COUNT],
    reg: u8,
) {
    if let Some(key) = values[reg as usize].take()
        && available.get(&key).copied() == Some(reg)
    {
        available.remove(&key);
    }
}

fn record_value_key(
    available: &mut HashMap<RegisterValueKey, u8>,
    values: &mut [Option<RegisterValueKey>; REG_COUNT],
    reg: u8,
    key: RegisterValueKey,
) {
    invalidate_value_key(available, values, reg);
    values[reg as usize] = Some(key);
    available.insert(key, reg);
}

fn coalesce_registers_block(insts: &mut [Instruction], start: usize, end: usize) -> bool {
    let mut changed = false;
    let mut available = HashMap::new();
    let mut values = [None; REG_COUNT];

    for inst in &mut insts[start..end] {
        if inst.removed {
            continue;
        }

        match inst.opcode {
            Opcode::LoadI => {
                let key = RegisterValueKey::Immediate(inst.sbx);
                if let Some(src) = available.get(&key).copied()
                    && src != inst.a
                {
                    *inst = Instruction::new_mov(inst.a, src);
                    changed = true;
                }
                record_value_key(&mut available, &mut values, inst.a, key);
            }
            Opcode::LoadK => {
                let key = RegisterValueKey::Constant(inst.bx);
                if let Some(src) = available.get(&key).copied()
                    && src != inst.a
                {
                    *inst = Instruction::new_mov(inst.a, src);
                    changed = true;
                }
                record_value_key(&mut available, &mut values, inst.a, key);
            }
            Opcode::Mov => {
                invalidate_value_key(&mut available, &mut values, inst.a);
                if let Some(key) = values[inst.b as usize] {
                    record_value_key(&mut available, &mut values, inst.a, key);
                }
            }
            Opcode::LoadAcc => {
                invalidate_value_key(&mut available, &mut values, ACC_REG);
                if let Some(key) = values[inst.a as usize] {
                    record_value_key(&mut available, &mut values, ACC_REG, key);
                }
            }
            Opcode::Add
            | Opcode::Eq
            | Opcode::Lt
            | Opcode::Lte
            | Opcode::StrictEq
            | Opcode::StrictNeq
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor
            | Opcode::Shl
            | Opcode::Shr
            | Opcode::Ushr
            | Opcode::Pow
            | Opcode::LogicalAnd
            | Opcode::LogicalOr
            | Opcode::NullishCoalesce
            | Opcode::In
            | Opcode::Instanceof
            | Opcode::AddStr
            | Opcode::AddAcc
            | Opcode::SubAcc
            | Opcode::MulAcc
            | Opcode::DivAcc
            | Opcode::AddStrAcc
            | Opcode::AddAccImm8
            | Opcode::SubAccImm8
            | Opcode::MulAccImm8
            | Opcode::DivAccImm8
            | Opcode::IncAcc
            | Opcode::LoadThis
            | Opcode::Load0
            | Opcode::Load1
            | Opcode::LoadNull
            | Opcode::LoadTrue
            | Opcode::LoadFalse
            | Opcode::Neg
            | Opcode::Inc
            | Opcode::Dec
            | Opcode::ToPrimitive
            | Opcode::BitNot
            | Opcode::Call1SubI
            | Opcode::Call2SubIAdd => {
                invalidate_value_key(&mut available, &mut values, ACC_REG);
            }
            _ => {
                available.clear();
                values.fill(None);
            }
        }
    }

    changed
}

fn classify_known_value(value: JSValue) -> KnownValueKind {
    if is_undefined(value) {
        KnownValueKind::Undefined
    } else if is_null(value) {
        KnownValueKind::Null
    } else {
        KnownValueKind::NonNullish
    }
}

fn bool_constant_index(
    constants: &mut Vec<JSValue>,
    value: bool,
    true_index: &mut Option<u16>,
    false_index: &mut Option<u16>,
) -> u16 {
    let slot = if value { true_index } else { false_index };
    if let Some(index) = *slot {
        return index;
    }

    let needle = if value { make_true() } else { make_false() };
    if let Some(index) = constants.iter().position(|constant| *constant == needle) {
        let index = index as u16;
        *slot = Some(index);
        return index;
    }

    let index = u16::try_from(constants.len()).expect("constant pool index must fit in u16");
    constants.push(needle);
    *slot = Some(index);
    index
}

fn literal_constant_index(constants: &mut Vec<JSValue>, value: JSValue) -> u16 {
    if let Some(index) = constants.iter().position(|constant| *constant == value) {
        return index as u16;
    }

    let index = u16::try_from(constants.len()).expect("constant pool index must fit in u16");
    constants.push(value);
    index
}

fn small_int_literal(value: JSValue) -> Option<i16> {
    let number = to_f64(value)?;
    if !number.is_finite() || number.fract() != 0.0 {
        return None;
    }
    let integer = number as i32;
    if !(i16::MIN as i32..=i16::MAX as i32).contains(&integer) {
        return None;
    }
    Some(integer as i16)
}

fn fold_add_literals(lhs: JSValue, rhs: JSValue) -> Option<i16> {
    let lhs = to_f64(lhs)?;
    let rhs = to_f64(rhs)?;
    if !lhs.is_finite() || !rhs.is_finite() || lhs.fract() != 0.0 || rhs.fract() != 0.0 {
        return None;
    }

    let sum = lhs as i32 + rhs as i32;
    if !(i16::MIN as i32..=i16::MAX as i32).contains(&sum) {
        return None;
    }

    Some(sum as i16)
}

fn fold_strict_eq_literals(lhs: JSValue, rhs: JSValue) -> Option<bool> {
    if let (Some(lhs), Some(rhs)) = (to_f64(lhs), to_f64(rhs)) {
        return Some(!lhs.is_nan() && !rhs.is_nan() && lhs == rhs);
    }

    let true_value = make_true();
    let false_value = make_false();
    if (lhs == true_value || lhs == false_value) && (rhs == true_value || rhs == false_value) {
        return Some(lhs == rhs);
    }

    if (is_null(lhs) || is_undefined(lhs)) && (is_null(rhs) || is_undefined(rhs)) {
        return Some(lhs == rhs);
    }

    None
}

fn fold_temporary_checks_block(
    insts: &mut [Instruction],
    constants: &mut Vec<JSValue>,
    start: usize,
    end: usize,
) -> bool {
    let mut changed = false;
    let mut known = [KnownValueKind::Unknown; REG_COUNT];
    let mut true_index = None;
    let mut false_index = None;

    for inst in &mut insts[start..end] {
        if inst.removed {
            continue;
        }

        match inst.opcode {
            Opcode::LoadI => {
                known[inst.a as usize] = KnownValueKind::NonNullish;
            }
            Opcode::LoadK => {
                known[inst.a as usize] = constants
                    .get(inst.bx as usize)
                    .copied()
                    .map(classify_known_value)
                    .unwrap_or(KnownValueKind::Unknown);
            }
            Opcode::Mov => {
                known[inst.a as usize] = known[inst.b as usize];
            }
            Opcode::LoadAcc => {
                known[ACC_REG as usize] = known[inst.a as usize];
            }
            Opcode::LoadThis
            | Opcode::Load0
            | Opcode::Load1
            | Opcode::LoadTrue
            | Opcode::LoadFalse => {
                known[ACC_REG as usize] = KnownValueKind::NonNullish;
            }
            Opcode::LoadNull => {
                known[ACC_REG as usize] = KnownValueKind::Null;
            }
            Opcode::IsUndef => {
                let replacement = match known[inst.b as usize] {
                    KnownValueKind::Undefined => Some(true),
                    KnownValueKind::Null | KnownValueKind::NonNullish => Some(false),
                    KnownValueKind::Unknown => None,
                };
                if let Some(value) = replacement {
                    let index =
                        bool_constant_index(constants, value, &mut true_index, &mut false_index);
                    *inst = Instruction::new_load_k(inst.a, index);
                    changed = true;
                }
                known[inst.a as usize] = KnownValueKind::NonNullish;
            }
            Opcode::IsNull => {
                let replacement = match known[inst.b as usize] {
                    KnownValueKind::Null => Some(true),
                    KnownValueKind::Undefined | KnownValueKind::NonNullish => Some(false),
                    KnownValueKind::Unknown => None,
                };
                if let Some(value) = replacement {
                    let index =
                        bool_constant_index(constants, value, &mut true_index, &mut false_index);
                    *inst = Instruction::new_load_k(inst.a, index);
                    changed = true;
                }
                known[inst.a as usize] = KnownValueKind::NonNullish;
            }
            _ => {
                known.fill(KnownValueKind::Unknown);
            }
        }
    }

    changed
}

fn rewrite_load_move(insts: &mut [Instruction], first: usize, second: usize) -> bool {
    if insts[first].opcode != Opcode::LoadI || insts[second].opcode != Opcode::Mov {
        return false;
    }
    if insts[second].b != insts[first].a {
        return false;
    }

    let value = insts[first].sbx;
    insts[second] = Instruction {
        opcode: Opcode::LoadI,
        a: insts[second].a,
        b: 0,
        c: 0,
        bx: value as u16,
        sbx: value,
        target: None,
        removed: false,
    };
    true
}

fn is_simple_call_arg_builder(inst: &Instruction, reserved: u8) -> bool {
    match inst.opcode {
        Opcode::LoadI
        | Opcode::LoadK
        | Opcode::LoadGlobalIc
        | Opcode::GetGlobal
        | Opcode::GetUpval
        | Opcode::ResolveScope
        | Opcode::NewObj
        | Opcode::NewArr
        | Opcode::NewFunc
        | Opcode::NewThis
        | Opcode::LoadClosure
        | Opcode::TypeofName
        | Opcode::CreateEnv
        | Opcode::LoadArg
        | Opcode::LoadRestArgs
        | Opcode::LoadName => inst.a != reserved,
        _ => false,
    }
}

fn fold_simple_arg_copy_into_call_method1(
    insts: &mut [Instruction],
    arg_builder: usize,
    obj_builder: usize,
    mov_index: usize,
    call_index: usize,
) -> bool {
    if insts[mov_index].opcode != Opcode::Mov || insts[call_index].opcode != Opcode::CallMethod1 {
        return false;
    }

    let obj_reg = insts[call_index].a;
    let Some(arg_reg) = obj_reg.checked_add(1) else {
        return false;
    };

    if insts[obj_builder].a != obj_reg
        || insts[mov_index].a != arg_reg
        || insts[mov_index].b != insts[arg_builder].a
    {
        return false;
    }

    if !is_simple_call_arg_builder(&insts[arg_builder], arg_reg)
        || !is_simple_call_arg_builder(&insts[obj_builder], arg_reg)
    {
        return false;
    }

    insts[arg_builder].a = arg_reg;
    insts[mov_index].removed = true;
    true
}

fn rewrite_move_into_sink(insts: &mut [Instruction], first: usize, second: usize) -> bool {
    if insts[first].opcode != Opcode::Mov {
        return false;
    }

    let dst = insts[first].a;
    let src = insts[first].b;
    let sink = &mut insts[second];
    if !matches!(
        sink.opcode,
        Opcode::StoreName | Opcode::InitName | Opcode::SetProp | Opcode::SetPropIc
    ) || sink.a != dst
    {
        return false;
    }

    sink.a = src;
    sink.removed = false;
    insts[first].removed = true;
    true
}

fn acc_used_before_redefined(insts: &[Instruction], start: usize, end: usize) -> bool {
    for index in start..end {
        let inst = &insts[index];
        if inst.removed {
            continue;
        }

        let operands = instruction_operands(&inst.decoded(index));
        if operands.uses.contains(&ACC_REG) {
            return true;
        }
        if operands.defs.contains(&ACC_REG) {
            return false;
        }
    }

    false
}

fn rewrite_name_store_load(
    insts: &mut [Instruction],
    first: usize,
    second: usize,
    end: usize,
) -> bool {
    if !matches!(insts[first].opcode, Opcode::StoreName | Opcode::InitName)
        || insts[second].opcode != Opcode::LoadName
        || insts[first].bx != insts[second].bx
    {
        return false;
    }

    let src = insts[first].a;
    let dst = insts[second].a;
    if acc_used_before_redefined(insts, second + 1, end) {
        if dst != src {
            return false;
        }

        insts[second] = Instruction::new_load_acc(src);
        return true;
    }

    if dst == src {
        insts[second].removed = true;
    } else {
        insts[second] = Instruction::new_mov(dst, src);
    }
    true
}

fn optimize_basic_peephole_block(insts: &mut [Instruction], start: usize, end: usize) -> bool {
    let mut changed = false;

    loop {
        let mut local_change = false;
        let live_indices: Vec<_> = (start..end)
            .filter(|&index| !insts[index].removed)
            .collect();

        for (pos, &index) in live_indices.iter().enumerate() {
            if insts[index].removed {
                continue;
            }

            if insts[index].opcode == Opcode::Mov && insts[index].a == insts[index].b {
                insts[index].removed = true;
                local_change = true;
                continue;
            }

            let Some(&next_index) = live_indices.get(pos + 1) else {
                continue;
            };
            if insts[next_index].removed {
                continue;
            }

            if insts[index].opcode == Opcode::Mov
                && insts[next_index].opcode == Opcode::Mov
                && insts[next_index].a == insts[index].b
                && insts[next_index].b == insts[index].a
            {
                insts[next_index].removed = true;
                local_change = true;
            }

            if insts[next_index].opcode == Opcode::Mov && insts[next_index].a == insts[next_index].b
            {
                insts[next_index].removed = true;
                local_change = true;
                continue;
            }

            if rewrite_load_move(insts, index, next_index) {
                local_change = true;
            }

            if let (Some(&third_index), Some(&fourth_index)) =
                (live_indices.get(pos + 2), live_indices.get(pos + 3))
                && !insts[third_index].removed
                && !insts[fourth_index].removed
                && (fold_simple_arg_copy_into_call_method1(
                    insts,
                    index,
                    next_index,
                    third_index,
                    fourth_index,
                ) || fold_simple_arg_copy_into_call_method1(
                    insts,
                    next_index,
                    index,
                    third_index,
                    fourth_index,
                ))
            {
                local_change = true;
            }

            if rewrite_move_into_sink(insts, index, next_index) {
                local_change = true;
            }

            if rewrite_name_store_load(insts, index, next_index, end) {
                local_change = true;
            }
        }

        if !local_change {
            break;
        }

        changed = true;
    }

    changed
}

fn resolve_alias(aliases: &[Option<u8>; REG_COUNT], reg: u8) -> u8 {
    let mut current = reg;
    let mut steps = 0usize;
    while steps < REG_COUNT {
        let Some(next) = aliases[current as usize] else {
            break;
        };
        if next == current {
            break;
        }
        current = next;
        steps += 1;
    }
    current
}

fn invalidate_alias(aliases: &mut [Option<u8>; REG_COUNT], reg: u8) {
    aliases[reg as usize] = None;
    for alias in aliases.iter_mut() {
        if *alias == Some(reg) {
            *alias = None;
        }
    }
}

fn rewrite_reg(aliases: &[Option<u8>; REG_COUNT], reg: &mut u8) -> bool {
    let resolved = resolve_alias(aliases, *reg);
    if resolved != *reg {
        *reg = resolved;
        true
    } else {
        false
    }
}

fn tracked_value_for_reg(
    reg: u8,
    aliases: &[Option<u8>; REG_COUNT],
    literals: &[Option<JSValue>; REG_COUNT],
) -> TrackedValue {
    let resolved = resolve_alias(aliases, reg);
    literals[resolved as usize]
        .map(TrackedValue::Literal)
        .unwrap_or(TrackedValue::Register(resolved))
}

fn invalidate_tracked_reg(
    aliases: &mut [Option<u8>; REG_COUNT],
    literals: &mut [Option<JSValue>; REG_COUNT],
    names: &mut HashMap<u16, TrackedValue>,
    properties: &mut HashMap<(u8, u8), TrackedValue>,
    reg: u8,
) {
    invalidate_alias(aliases, reg);
    literals[reg as usize] = None;
    names.retain(|_, value| !value.uses_register(reg));
    properties.retain(|(object, _), value| *object != reg && !value.uses_register(reg));
}

fn clear_tracked_state(
    aliases: &mut [Option<u8>; REG_COUNT],
    literals: &mut [Option<JSValue>; REG_COUNT],
    names: &mut HashMap<u16, TrackedValue>,
    properties: &mut HashMap<(u8, u8), TrackedValue>,
) {
    aliases.fill(None);
    literals.fill(None);
    names.clear();
    properties.clear();
}

fn assign_tracked_value(
    reg: u8,
    value: TrackedValue,
    aliases: &mut [Option<u8>; REG_COUNT],
    literals: &mut [Option<JSValue>; REG_COUNT],
    names: &mut HashMap<u16, TrackedValue>,
    properties: &mut HashMap<(u8, u8), TrackedValue>,
) {
    invalidate_tracked_reg(aliases, literals, names, properties, reg);
    match value {
        TrackedValue::Register(source) => {
            let source = resolve_alias(aliases, source);
            if reg != source {
                aliases[reg as usize] = Some(source);
            }
            literals[reg as usize] = literals[source as usize];
        }
        TrackedValue::Literal(value) => {
            literals[reg as usize] = Some(value);
        }
    }
}

fn tracked_value_matches_reg(
    reg: u8,
    value: TrackedValue,
    aliases: &[Option<u8>; REG_COUNT],
    literals: &[Option<JSValue>; REG_COUNT],
) -> bool {
    match value {
        TrackedValue::Register(source) => {
            resolve_alias(aliases, reg) == resolve_alias(aliases, source)
        }
        TrackedValue::Literal(value) => {
            literals[resolve_alias(aliases, reg) as usize] == Some(value)
        }
    }
}

fn materialize_value_preserving_acc(
    value: TrackedValue,
    dst: u8,
    aliases: &[Option<u8>; REG_COUNT],
    literals: &[Option<JSValue>; REG_COUNT],
    constants: &mut Vec<JSValue>,
) -> Option<Instruction> {
    if tracked_value_matches_reg(dst, value, aliases, literals) {
        return None;
    }

    match value {
        TrackedValue::Register(source) => Some(Instruction::new_mov(dst, source)),
        TrackedValue::Literal(value) => Some(
            small_int_literal(value)
                .map(|literal| Instruction::new_load_i(dst, literal))
                .unwrap_or_else(|| {
                    Instruction::new_load_k(dst, literal_constant_index(constants, value))
                }),
        ),
    }
}

fn materialize_name_load(
    value: TrackedValue,
    dst: u8,
    acc_used: bool,
    aliases: &[Option<u8>; REG_COUNT],
    literals: &[Option<JSValue>; REG_COUNT],
    constants: &mut Vec<JSValue>,
) -> Option<Instruction> {
    if !acc_used {
        return materialize_value_preserving_acc(value, dst, aliases, literals, constants);
    }

    match value {
        TrackedValue::Register(source) if source == dst => Some(Instruction::new_load_acc(source)),
        TrackedValue::Register(_) => None,
        TrackedValue::Literal(value) if value == make_true() => {
            Some(Instruction::new_load_true(dst))
        }
        TrackedValue::Literal(value) if value == make_false() => {
            Some(Instruction::new_load_false(dst))
        }
        TrackedValue::Literal(value) if is_null(value) => Some(Instruction::new_load_null(dst)),
        TrackedValue::Literal(value) if dst == ACC_REG => Some(
            small_int_literal(value)
                .map(|literal| Instruction::new_load_i(ACC_REG, literal))
                .unwrap_or_else(|| {
                    Instruction::new_load_k(ACC_REG, literal_constant_index(constants, value))
                }),
        ),
        TrackedValue::Literal(_) => None,
    }
}

fn copy_propagation_block(
    insts: &mut [Instruction],
    constants: &mut Vec<JSValue>,
    start: usize,
    end: usize,
) -> bool {
    let mut changed = false;
    let mut aliases = [None; REG_COUNT];
    let mut literals = [None; REG_COUNT];
    let mut names = HashMap::new();
    let mut properties = HashMap::new();

    for index in start..end {
        if insts[index].removed {
            continue;
        }

        match insts[index].opcode {
            Opcode::Mov => {
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                let value = tracked_value_for_reg(insts[index].b, &aliases, &literals);
                assign_tracked_value(
                    insts[index].a,
                    value,
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                );
            }
            Opcode::Add
            | Opcode::Eq
            | Opcode::Lt
            | Opcode::Lte
            | Opcode::StrictEq
            | Opcode::StrictNeq
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor
            | Opcode::Shl
            | Opcode::Shr
            | Opcode::Ushr
            | Opcode::Pow
            | Opcode::LogicalAnd
            | Opcode::LogicalOr
            | Opcode::NullishCoalesce
            | Opcode::In
            | Opcode::Instanceof
            | Opcode::AddStr
            | Opcode::EqI32Fast
            | Opcode::LtI32Fast => {
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                changed |= rewrite_reg(&aliases, &mut insts[index].c);

                if insts[index].opcode == Opcode::Add
                    && let (Some(lhs), Some(rhs)) = (
                        literals[insts[index].b as usize],
                        literals[insts[index].c as usize],
                    )
                    && let Some(folded) = fold_add_literals(lhs, rhs)
                {
                    insts[index] = Instruction::new_load_i(ACC_REG, folded);
                    assign_tracked_value(
                        ACC_REG,
                        TrackedValue::Literal(make_int32(folded as i32)),
                        &mut aliases,
                        &mut literals,
                        &mut names,
                        &mut properties,
                    );
                    changed = true;
                    continue;
                }

                if insts[index].opcode == Opcode::StrictEq
                    && let (Some(lhs), Some(rhs)) = (
                        literals[insts[index].b as usize],
                        literals[insts[index].c as usize],
                    )
                    && let Some(result) = fold_strict_eq_literals(lhs, rhs)
                {
                    insts[index] = if result {
                        Instruction::new_load_true(ACC_REG)
                    } else {
                        Instruction::new_load_false(ACC_REG)
                    };
                    assign_tracked_value(
                        ACC_REG,
                        TrackedValue::Literal(if result { make_true() } else { make_false() }),
                        &mut aliases,
                        &mut literals,
                        &mut names,
                        &mut properties,
                    );
                    changed = true;
                    continue;
                }

                invalidate_tracked_reg(
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                    ACC_REG,
                );
            }
            Opcode::AddAcc
            | Opcode::SubAcc
            | Opcode::MulAcc
            | Opcode::DivAcc
            | Opcode::AddStrAcc
            | Opcode::Neg
            | Opcode::Inc
            | Opcode::Dec
            | Opcode::ToPrimitive
            | Opcode::BitNot => {
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                invalidate_tracked_reg(
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                    ACC_REG,
                );
            }
            Opcode::Typeof
            | Opcode::ToNum
            | Opcode::ToStr
            | Opcode::IsUndef
            | Opcode::IsNull
            | Opcode::LoadArg
            | Opcode::LoadRestArgs
            | Opcode::GetScope
            | Opcode::SetScope => {
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                invalidate_tracked_reg(
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                    insts[index].a,
                );
            }
            Opcode::GetProp | Opcode::GetPropIc => {
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                let object = resolve_alias(&aliases, insts[index].b);
                if let Some(value) = properties.get(&(object, insts[index].c)).copied() {
                    match materialize_value_preserving_acc(
                        value,
                        insts[index].a,
                        &aliases,
                        &literals,
                        constants,
                    ) {
                        Some(replacement) => {
                            if insts[index].opcode != replacement.opcode
                                || insts[index].a != replacement.a
                                || insts[index].b != replacement.b
                                || insts[index].c != replacement.c
                                || insts[index].bx != replacement.bx
                                || insts[index].sbx != replacement.sbx
                            {
                                insts[index] = replacement;
                                changed = true;
                            }
                            assign_tracked_value(
                                insts[index].a,
                                value,
                                &mut aliases,
                                &mut literals,
                                &mut names,
                                &mut properties,
                            );
                        }
                        None => {
                            insts[index].removed = true;
                            changed = true;
                        }
                    }
                } else {
                    invalidate_tracked_reg(
                        &mut aliases,
                        &mut literals,
                        &mut names,
                        &mut properties,
                        insts[index].a,
                    );
                }
            }
            Opcode::DeleteProp => {
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                let object = resolve_alias(&aliases, insts[index].b);
                properties.retain(|(tracked_object, _), _| *tracked_object != object);
                invalidate_tracked_reg(
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                    insts[index].a,
                );
            }
            Opcode::GetSuper | Opcode::HasProp | Opcode::GetLengthIc => {
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                invalidate_tracked_reg(
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                    insts[index].a,
                );
            }
            Opcode::SetProp | Opcode::SetPropIc => {
                changed |= rewrite_reg(&aliases, &mut insts[index].a);
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                let value = tracked_value_for_reg(insts[index].a, &aliases, &literals);
                properties.insert(
                    (resolve_alias(&aliases, insts[index].b), insts[index].c),
                    value,
                );
                assign_tracked_value(
                    ACC_REG,
                    value,
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                );
            }
            Opcode::SetSuper => {
                changed |= rewrite_reg(&aliases, &mut insts[index].a);
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                clear_tracked_state(&mut aliases, &mut literals, &mut names, &mut properties);
            }
            Opcode::GetIdxFast | Opcode::GetIdxIc => {
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                changed |= rewrite_reg(&aliases, &mut insts[index].c);
                invalidate_tracked_reg(
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                    insts[index].a,
                );
            }
            Opcode::SetIdxFast | Opcode::SetIdxIc => {
                changed |= rewrite_reg(&aliases, &mut insts[index].a);
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                changed |= rewrite_reg(&aliases, &mut insts[index].c);
                clear_tracked_state(&mut aliases, &mut literals, &mut names, &mut properties);
            }
            Opcode::LoadAcc => {
                changed |= rewrite_reg(&aliases, &mut insts[index].a);
                let value = tracked_value_for_reg(insts[index].a, &aliases, &literals);
                assign_tracked_value(
                    ACC_REG,
                    value,
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                );
            }
            Opcode::StoreName | Opcode::InitName => {
                changed |= rewrite_reg(&aliases, &mut insts[index].a);
                names.insert(
                    insts[index].bx,
                    tracked_value_for_reg(insts[index].a, &aliases, &literals),
                );
            }
            Opcode::JmpTrue
            | Opcode::JmpFalse
            | Opcode::Yield
            | Opcode::Await
            | Opcode::Throw
            | Opcode::RetReg
            | Opcode::TestJmpTrue => {
                changed |= rewrite_reg(&aliases, &mut insts[index].a);
            }
            Opcode::JmpEq
            | Opcode::JmpNeq
            | Opcode::JmpLt
            | Opcode::JmpLtF64
            | Opcode::JmpLte
            | Opcode::JmpLteF64
            | Opcode::JmpLteFalse
            | Opcode::JmpLteFalseF64
            | Opcode::JmpI32Fast
            | Opcode::CmpJmp
            | Opcode::LoadCmpEqJfalse
            | Opcode::LoadCmpLtJfalse => {
                changed |= rewrite_reg(&aliases, &mut insts[index].a);
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
            }
            Opcode::EqJmpTrue | Opcode::LtJmp | Opcode::EqJmpFalse | Opcode::LteJmpLoop => {
                changed |= rewrite_reg(&aliases, &mut insts[index].b);
                changed |= rewrite_reg(&aliases, &mut insts[index].c);
            }
            Opcode::LoadI => {
                invalidate_tracked_reg(
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                    insts[index].a,
                );
                literals[insts[index].a as usize] = Some(make_int32(insts[index].sbx as i32));
            }
            Opcode::LoadK => {
                invalidate_tracked_reg(
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                    insts[index].a,
                );
                literals[insts[index].a as usize] =
                    constants.get(insts[index].bx as usize).copied();
            }
            Opcode::Load0 => {
                assign_tracked_value(
                    ACC_REG,
                    TrackedValue::Literal(make_int32(0)),
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                );
            }
            Opcode::Load1 => {
                assign_tracked_value(
                    ACC_REG,
                    TrackedValue::Literal(make_int32(1)),
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                );
            }
            Opcode::LoadTrue => {
                let value = TrackedValue::Literal(make_true());
                assign_tracked_value(
                    ACC_REG,
                    value,
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                );
                if insts[index].a != ACC_REG {
                    assign_tracked_value(
                        insts[index].a,
                        value,
                        &mut aliases,
                        &mut literals,
                        &mut names,
                        &mut properties,
                    );
                }
            }
            Opcode::LoadFalse => {
                let value = TrackedValue::Literal(make_false());
                assign_tracked_value(
                    ACC_REG,
                    value,
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                );
                if insts[index].a != ACC_REG {
                    assign_tracked_value(
                        insts[index].a,
                        value,
                        &mut aliases,
                        &mut literals,
                        &mut names,
                        &mut properties,
                    );
                }
            }
            Opcode::LoadNull => {
                let value = TrackedValue::Literal(make_null());
                assign_tracked_value(
                    ACC_REG,
                    value,
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                );
                if insts[index].a != ACC_REG {
                    assign_tracked_value(
                        insts[index].a,
                        value,
                        &mut aliases,
                        &mut literals,
                        &mut names,
                        &mut properties,
                    );
                }
            }
            Opcode::LoadGlobalIc
            | Opcode::GetGlobal
            | Opcode::GetUpval
            | Opcode::NewObj
            | Opcode::NewArr
            | Opcode::NewFunc
            | Opcode::NewClass
            | Opcode::NewThis
            | Opcode::LoadClosure
            | Opcode::ResolveScope
            | Opcode::TypeofName
            | Opcode::CreateEnv
            | Opcode::Keys
            | Opcode::ForIn
            | Opcode::IteratorNext => {
                invalidate_tracked_reg(
                    &mut aliases,
                    &mut literals,
                    &mut names,
                    &mut properties,
                    insts[index].a,
                );
            }
            Opcode::LoadName => {
                let tracked = names.get(&insts[index].bx).copied();
                if let Some(value) = tracked {
                    let acc_used = acc_used_before_redefined(insts, index + 1, end);
                    match materialize_name_load(
                        value,
                        insts[index].a,
                        acc_used,
                        &aliases,
                        &literals,
                        constants,
                    ) {
                        Some(replacement) => {
                            if insts[index].opcode != replacement.opcode
                                || insts[index].a != replacement.a
                                || insts[index].b != replacement.b
                                || insts[index].c != replacement.c
                                || insts[index].bx != replacement.bx
                                || insts[index].sbx != replacement.sbx
                            {
                                insts[index] = replacement;
                                changed = true;
                            }

                            match insts[index].opcode {
                                Opcode::LoadAcc => {
                                    assign_tracked_value(
                                        ACC_REG,
                                        value,
                                        &mut aliases,
                                        &mut literals,
                                        &mut names,
                                        &mut properties,
                                    );
                                }
                                Opcode::LoadTrue | Opcode::LoadFalse | Opcode::LoadNull => {
                                    assign_tracked_value(
                                        ACC_REG,
                                        value,
                                        &mut aliases,
                                        &mut literals,
                                        &mut names,
                                        &mut properties,
                                    );
                                    if insts[index].a != ACC_REG {
                                        assign_tracked_value(
                                            insts[index].a,
                                            value,
                                            &mut aliases,
                                            &mut literals,
                                            &mut names,
                                            &mut properties,
                                        );
                                    }
                                }
                                _ => {
                                    assign_tracked_value(
                                        insts[index].a,
                                        value,
                                        &mut aliases,
                                        &mut literals,
                                        &mut names,
                                        &mut properties,
                                    );
                                    if !acc_used {
                                        invalidate_tracked_reg(
                                            &mut aliases,
                                            &mut literals,
                                            &mut names,
                                            &mut properties,
                                            ACC_REG,
                                        );
                                    } else if insts[index].a == ACC_REG {
                                        assign_tracked_value(
                                            ACC_REG,
                                            value,
                                            &mut aliases,
                                            &mut literals,
                                            &mut names,
                                            &mut properties,
                                        );
                                    }
                                }
                            }
                        }
                        None if !acc_used
                            && tracked_value_matches_reg(
                                insts[index].a,
                                value,
                                &aliases,
                                &literals,
                            ) =>
                        {
                            insts[index].removed = true;
                            changed = true;
                        }
                        None => {
                            assign_tracked_value(
                                insts[index].a,
                                value,
                                &mut aliases,
                                &mut literals,
                                &mut names,
                                &mut properties,
                            );
                            assign_tracked_value(
                                ACC_REG,
                                value,
                                &mut aliases,
                                &mut literals,
                                &mut names,
                                &mut properties,
                            );
                        }
                    }
                } else {
                    invalidate_tracked_reg(
                        &mut aliases,
                        &mut literals,
                        &mut names,
                        &mut properties,
                        insts[index].a,
                    );
                    invalidate_tracked_reg(
                        &mut aliases,
                        &mut literals,
                        &mut names,
                        &mut properties,
                        ACC_REG,
                    );
                }
            }
            Opcode::Ret | Opcode::RetU | Opcode::Jmp | Opcode::Switch | Opcode::LoopHint => {}
            _ => {
                clear_tracked_state(&mut aliases, &mut literals, &mut names, &mut properties);
            }
        }
    }

    changed
}

fn resolve_threaded_target(insts: &[Instruction], mut target: usize) -> usize {
    let mut seen = BTreeSet::new();

    while target < insts.len() {
        if !seen.insert(target) {
            break;
        }

        let Some(inst) = insts.get(target) else {
            break;
        };

        if inst.removed {
            target += 1;
            continue;
        }

        if inst.opcode == Opcode::Jmp
            && let Some(next) = inst.target
        {
            target = next;
            continue;
        }

        break;
    }

    target
}

fn thread_jumps(insts: &mut [Instruction]) -> bool {
    let mut changed = false;

    for index in 0..insts.len() {
        if insts[index].removed || insts[index].opcode != Opcode::Jmp {
            continue;
        }

        let Some(target) = insts[index].target else {
            continue;
        };
        let threaded = resolve_threaded_target(insts, target);
        if threaded != target {
            insts[index].target = Some(threaded);
            changed = true;
        }

        if insts[index].target == Some(index + 1) {
            insts[index].removed = true;
            changed = true;
        }
    }

    changed
}

fn normal_successors(pc: usize, len: usize) -> Vec<usize> {
    if pc + 1 < len {
        vec![pc + 1]
    } else {
        Vec::new()
    }
}

fn target_successors(target: Option<usize>, len: usize) -> Vec<usize> {
    target
        .filter(|&target| target < len)
        .into_iter()
        .collect::<Vec<_>>()
}

fn conditional_successors(pc: usize, target: Option<usize>, len: usize) -> Vec<usize> {
    let mut successors = target_successors(target, len);
    if pc + 1 < len && !successors.contains(&(pc + 1)) {
        successors.push(pc + 1);
    }
    successors
}

fn push_unique_reg(regs: &mut Vec<u8>, reg: u8) {
    if !regs.contains(&reg) {
        regs.push(reg);
    }
}

fn push_call_bundle(regs: &mut Vec<u8>, base: u8, arg_count: u8) -> bool {
    let last = base as usize + arg_count as usize;
    if last >= ACC_REG as usize {
        return false;
    }

    for reg in base..=base + arg_count {
        push_unique_reg(regs, reg);
    }

    true
}

fn build_instruction_semantics(
    insts: &[Instruction],
    constants: &[JSValue],
) -> Option<Vec<InstructionSemantics>> {
    let len = insts.len();
    let mut semantics = Vec::with_capacity(len);

    for (pc, inst) in insts.iter().enumerate() {
        let ops = instruction_operands(&inst.decoded(pc));
        let mut pinned = Vec::new();
        let mut successors = normal_successors(pc, len);

        match inst.opcode {
            Opcode::Jmp => {
                successors = target_successors(inst.target, len);
            }
            Opcode::JmpTrue
            | Opcode::JmpFalse
            | Opcode::JmpEq
            | Opcode::JmpNeq
            | Opcode::JmpLt
            | Opcode::JmpLtF64
            | Opcode::JmpLte
            | Opcode::JmpLteF64
            | Opcode::JmpLteFalse
            | Opcode::JmpLteFalseF64
            | Opcode::JmpI32Fast
            | Opcode::CmpJmp
            | Opcode::LoadCmpEqJfalse
            | Opcode::LoadCmpLtJfalse
            | Opcode::TestJmpTrue => {
                successors = conditional_successors(pc, inst.target, len);
            }
            Opcode::EqJmpTrue | Opcode::LtJmp | Opcode::EqJmpFalse | Opcode::LteJmpLoop => {
                successors = conditional_successors(pc, inst.target, len);
            }
            Opcode::LoopIncJmp | Opcode::IncJmpFalseLoop | Opcode::IncAccJmp => {
                successors = conditional_successors(pc, inst.target, len);
            }
            Opcode::Switch => {
                successors = switch_targets(pc, inst.b as usize, constants);
                if successors.is_empty() {
                    successors = normal_successors(pc, len);
                }
            }
            Opcode::Ret | Opcode::RetReg | Opcode::RetU | Opcode::Throw | Opcode::CallRet => {
                successors.clear();
            }
            Opcode::Call
            | Opcode::TailCall
            | Opcode::Construct
            | Opcode::CallIc
            | Opcode::CallIcSuper
            | Opcode::CallThis
            | Opcode::CallMono => {
                if !push_call_bundle(
                    &mut pinned,
                    inst.a,
                    if inst.opcode == Opcode::CallThis {
                        inst.c
                    } else {
                        inst.b
                    },
                ) {
                    return None;
                }
            }
            Opcode::ProfileHotCall => {
                if !push_call_bundle(&mut pinned, inst.b, inst.c) {
                    return None;
                }
            }
            Opcode::CallVar | Opcode::CallIcVar => {
                if inst.a as usize + 1 >= ACC_REG as usize {
                    return None;
                }
                push_unique_reg(&mut pinned, inst.a);
                push_unique_reg(&mut pinned, inst.a + 1);
            }
            Opcode::CallThisVar => {
                push_unique_reg(&mut pinned, inst.a);
                push_unique_reg(&mut pinned, inst.b);
                push_unique_reg(&mut pinned, inst.c);
            }
            _ => {}
        }

        semantics.push(InstructionSemantics {
            uses: ops.uses,
            defs: ops.defs,
            successors,
            pinned,
        });
    }

    Some(semantics)
}

fn union_live_sets(dst: &mut [bool; REG_COUNT], src: &[bool; REG_COUNT]) {
    for index in 0..REG_COUNT {
        dst[index] |= src[index];
    }
}

fn extend_interval(intervals: &mut [Option<LiveInterval>; REG_COUNT], reg: usize, pc: usize) {
    match &mut intervals[reg] {
        Some(interval) => {
            interval.start = interval.start.min(pc);
            interval.end = interval.end.max(pc);
        }
        slot @ None => {
            *slot = Some(LiveInterval {
                reg: reg as u8,
                start: pc,
                end: pc,
            });
        }
    }
}

fn analyze_liveness(insts: &[Instruction], constants: &[JSValue]) -> Option<LivenessAnalysis> {
    let semantics = build_instruction_semantics(insts, constants)?;
    let mut live_in = vec![[false; REG_COUNT]; insts.len()];
    let mut live_out = vec![[false; REG_COUNT]; insts.len()];

    loop {
        let mut changed = false;

        for pc in (0..insts.len()).rev() {
            let mut next_out = [false; REG_COUNT];
            for &succ in &semantics[pc].successors {
                union_live_sets(&mut next_out, &live_in[succ]);
            }

            let mut next_in = next_out;
            for &def in &semantics[pc].defs {
                next_in[def as usize] = false;
            }
            for &use_reg in &semantics[pc].uses {
                next_in[use_reg as usize] = true;
            }

            if live_out[pc] != next_out {
                live_out[pc] = next_out;
                changed = true;
            }
            if live_in[pc] != next_in {
                live_in[pc] = next_in;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    let mut intervals = [None; REG_COUNT];
    let mut pinned = [false; REG_COUNT];
    pinned[0] = true;
    pinned[ACC_REG as usize] = true;

    for (pc, info) in semantics.iter().enumerate() {
        for &reg in &info.pinned {
            pinned[reg as usize] = true;
        }

        for reg in 0..REG_COUNT {
            if live_in[pc][reg] || live_out[pc][reg] {
                extend_interval(&mut intervals, reg, pc);
            }
        }

        for &reg in &info.uses {
            extend_interval(&mut intervals, reg as usize, pc);
        }
        for &reg in &info.defs {
            extend_interval(&mut intervals, reg as usize, pc);
        }
    }

    Some(LivenessAnalysis { intervals, pinned })
}

fn rewrite_register_with_map(reg: &mut u8, map: &[u8; REG_COUNT]) {
    *reg = map[*reg as usize];
}

fn rewrite_instruction_registers(inst: &mut Instruction, map: &[u8; REG_COUNT]) -> bool {
    match inst.opcode {
        Opcode::DeleteProp
        | Opcode::HasProp
        | Opcode::GetProp
        | Opcode::GetSuper
        | Opcode::GetPropIc
        | Opcode::GetPropMono
        | Opcode::GetPropIcMov
        | Opcode::NewObjInitProp
        | Opcode::Spread
        | Opcode::SetProp
        | Opcode::SetSuper
        | Opcode::SetPropIc
        | Opcode::GetIdxFast
        | Opcode::GetIdxIc
        | Opcode::GetElem
        | Opcode::SetIdxFast
        | Opcode::SetIdxIc
        | Opcode::SetElem
        | Opcode::CallMethod1
        | Opcode::CallMethod2
        | Opcode::CallMethodIc
        | Opcode::CallMethod2Ic
        | Opcode::GetPropAcc
        | Opcode::SetPropAcc
        | Opcode::LoadGetProp
        | Opcode::LoadGetPropCmpEq
        | Opcode::GetLengthIc
        | Opcode::GetLengthIcCall
        | Opcode::GetPropAccCall
        | Opcode::GetPropIcCall
        | Opcode::GetPropCall
        | Opcode::GetPropAddImmSetPropIc
        | Opcode::GetProp2Ic
        | Opcode::GetProp3Ic
        | Opcode::GetPropElem
        | Opcode::GetPropChainAcc => return false,
        Opcode::Mov => {
            rewrite_register_with_map(&mut inst.a, map);
            rewrite_register_with_map(&mut inst.b, map);
        }
        Opcode::LoadI
        | Opcode::LoadK
        | Opcode::LoadGlobalIc
        | Opcode::GetGlobal
        | Opcode::GetUpval
        | Opcode::GetScope
        | Opcode::ResolveScope
        | Opcode::NewObj
        | Opcode::NewArr
        | Opcode::NewFunc
        | Opcode::NewThis
        | Opcode::LoadClosure
        | Opcode::TypeofName
        | Opcode::CreateEnv
        | Opcode::LoadArg
        | Opcode::LoadRestArgs
        | Opcode::IteratorNext
        | Opcode::SetGlobalIc
        | Opcode::SetGlobal
        | Opcode::SetUpval
        | Opcode::SetScope
        | Opcode::StoreName
        | Opcode::InitName
        | Opcode::LoadName
        | Opcode::LoadAcc
        | Opcode::JmpTrue
        | Opcode::JmpFalse
        | Opcode::TestJmpTrue
        | Opcode::RetReg
        | Opcode::Yield
        | Opcode::Await
        | Opcode::ArrayPushAcc
        | Opcode::LoadKCmp
        | Opcode::Call0
        | Opcode::AddAccImm8Mov
        | Opcode::LoadArgCall
        | Opcode::IncJmpFalseLoop
        | Opcode::LoadKAdd => {
            rewrite_register_with_map(&mut inst.a, map);
        }
        Opcode::NewClass
        | Opcode::Typeof
        | Opcode::ToNum
        | Opcode::ToStr
        | Opcode::IsUndef
        | Opcode::IsNull
        | Opcode::Keys => {
            rewrite_register_with_map(&mut inst.a, map);
            rewrite_register_with_map(&mut inst.b, map);
        }
        Opcode::ForIn => {
            rewrite_register_with_map(&mut inst.a, map);
            rewrite_register_with_map(&mut inst.b, map);
        }
        Opcode::Add
        | Opcode::Eq
        | Opcode::Lt
        | Opcode::Lte
        | Opcode::StrictEq
        | Opcode::StrictNeq
        | Opcode::BitAnd
        | Opcode::BitOr
        | Opcode::BitXor
        | Opcode::Shl
        | Opcode::Shr
        | Opcode::Ushr
        | Opcode::Pow
        | Opcode::LogicalAnd
        | Opcode::LogicalOr
        | Opcode::NullishCoalesce
        | Opcode::In
        | Opcode::Instanceof
        | Opcode::AddStr
        | Opcode::EqI32Fast
        | Opcode::LtI32Fast
        | Opcode::JmpEq
        | Opcode::JmpNeq
        | Opcode::JmpLt
        | Opcode::JmpLtF64
        | Opcode::JmpLte
        | Opcode::JmpLteF64
        | Opcode::JmpLteFalse
        | Opcode::JmpLteFalseF64
        | Opcode::JmpI32Fast
        | Opcode::EqJmpTrue
        | Opcode::LtJmp
        | Opcode::EqJmpFalse
        | Opcode::LteJmpLoop
        | Opcode::Call1
        | Opcode::LoadInc
        | Opcode::LoadDec => {
            rewrite_register_with_map(&mut inst.a, map);
            rewrite_register_with_map(&mut inst.b, map);
            if !matches!(inst.opcode, Opcode::LoadInc | Opcode::LoadDec) {
                rewrite_register_with_map(&mut inst.c, map);
            }
        }
        Opcode::Call2
        | Opcode::AddMov
        | Opcode::LoadAdd
        | Opcode::LoadSub
        | Opcode::LoadMul
        | Opcode::LoadCmpEq
        | Opcode::LoadCmpLt
        | Opcode::RetIfLteI
        | Opcode::Call2Add
        | Opcode::AddAccReg => {
            rewrite_register_with_map(&mut inst.a, map);
            rewrite_register_with_map(&mut inst.b, map);
            rewrite_register_with_map(&mut inst.c, map);
        }
        Opcode::AddAcc
        | Opcode::SubAcc
        | Opcode::MulAcc
        | Opcode::DivAcc
        | Opcode::AddStrAcc
        | Opcode::Neg
        | Opcode::Inc
        | Opcode::Dec
        | Opcode::ToPrimitive
        | Opcode::BitNot
        | Opcode::AddStrAccMov
        | Opcode::MulAccMov
        | Opcode::ProfileHotCall => {
            rewrite_register_with_map(&mut inst.b, map);
            if matches!(inst.opcode, Opcode::AddStrAccMov | Opcode::MulAccMov) {
                rewrite_register_with_map(&mut inst.a, map);
            }
        }
        Opcode::AddI
        | Opcode::SubI
        | Opcode::MulI
        | Opcode::DivI
        | Opcode::ModI
        | Opcode::Mod
        | Opcode::AddI32
        | Opcode::AddF64
        | Opcode::SubI32
        | Opcode::SubF64
        | Opcode::MulI32
        | Opcode::MulF64
        | Opcode::AddI32Fast
        | Opcode::AddF64Fast
        | Opcode::SubI32Fast
        | Opcode::MulI32Fast => {
            rewrite_register_with_map(&mut inst.a, map);
            rewrite_register_with_map(&mut inst.b, map);
            if !matches!(
                inst.opcode,
                Opcode::AddI | Opcode::SubI | Opcode::MulI | Opcode::DivI | Opcode::ModI
            ) {
                rewrite_register_with_map(&mut inst.c, map);
            }
        }
        Opcode::LoopIncJmp | Opcode::Call1SubI | Opcode::Call2SubIAdd | Opcode::Call1Add => {
            rewrite_register_with_map(&mut inst.a, map);
            rewrite_register_with_map(&mut inst.b, map);
        }
        Opcode::ProfileType | Opcode::ProfileCall | Opcode::CheckType | Opcode::CheckStruct => {
            if inst.b != 0 || inst.c != 0 {
                rewrite_register_with_map(&mut inst.b, map);
            }
        }
        Opcode::CheckIc | Opcode::IcInit | Opcode::IcUpdate => {
            if inst.b != 0 || inst.c != 0 {
                rewrite_register_with_map(&mut inst.b, map);
            }
        }
        Opcode::SafetyCheck => {
            if inst.a != 0 {
                rewrite_register_with_map(&mut inst.a, map);
            }
        }
        Opcode::Call
        | Opcode::TailCall
        | Opcode::Construct
        | Opcode::CallIc
        | Opcode::CallIcSuper
        | Opcode::CallMono
        | Opcode::CallRet
        | Opcode::CallVar
        | Opcode::CallIcVar
        | Opcode::CallThis
        | Opcode::CallThisVar => {
            rewrite_register_with_map(&mut inst.a, map);
            if matches!(inst.opcode, Opcode::CallThis | Opcode::CallThisVar) {
                rewrite_register_with_map(&mut inst.b, map);
            }
            if matches!(inst.opcode, Opcode::CallThisVar) {
                rewrite_register_with_map(&mut inst.c, map);
            }
        }
        Opcode::LoadThis
        | Opcode::Load0
        | Opcode::Load1
        | Opcode::LoadNull
        | Opcode::LoadTrue
        | Opcode::LoadFalse
        | Opcode::AddAccImm8
        | Opcode::SubAccImm8
        | Opcode::MulAccImm8
        | Opcode::DivAccImm8
        | Opcode::IncAcc
        | Opcode::Ret
        | Opcode::RetU
        | Opcode::Jmp
        | Opcode::Switch
        | Opcode::LoopHint
        | Opcode::ProfileRet
        | Opcode::IcMiss
        | Opcode::OsrEntry
        | Opcode::ProfileHotLoop
        | Opcode::OsrExit
        | Opcode::JitHint
        | Opcode::Enter
        | Opcode::Leave
        | Opcode::LoadKAddAcc
        | Opcode::LoadKMulAcc
        | Opcode::LoadKSubAcc
        | Opcode::LoadThisCall
        | Opcode::IncAccJmp
        | Opcode::AssertValue
        | Opcode::AssertOk
        | Opcode::AssertFail
        | Opcode::AssertThrows
        | Opcode::AssertDoesNotThrow
        | Opcode::AssertRejects
        | Opcode::AssertDoesNotReject
        | Opcode::AssertEqual
        | Opcode::AssertNotEqual
        | Opcode::AssertDeepEqual
        | Opcode::AssertNotDeepEqual
        | Opcode::AssertStrictEqual
        | Opcode::AssertNotStrictEqual
        | Opcode::AssertDeepStrictEqual
        | Opcode::AssertNotDeepStrictEqual => {}
        Opcode::Reserved(_)
        | Opcode::CmpJmp
        | Opcode::LoadJfalse
        | Opcode::LoadCmpEqJfalse
        | Opcode::LoadCmpLtJfalse
        | Opcode::Destructure
        | Opcode::Throw
        | Opcode::Try
        | Opcode::EndTry
        | Opcode::Catch
        | Opcode::Finally => {}
        _ => {}
    }

    true
}

fn reuse_registers_linear_scan(insts: &mut [Instruction], constants: &[JSValue]) -> bool {
    let Some(liveness) = analyze_liveness(insts, constants) else {
        return false;
    };

    let mut map = [0u8; REG_COUNT];
    for (index, slot) in map.iter_mut().enumerate() {
        *slot = index as u8;
    }

    let mut available_regs = Vec::new();
    for reg in 1..ACC_REG {
        if !liveness.pinned[reg as usize] {
            available_regs.push(reg);
        }
    }

    let mut intervals = liveness
        .intervals
        .iter()
        .flatten()
        .copied()
        .filter(|interval| {
            interval.reg != 0 && interval.reg != ACC_REG && !liveness.pinned[interval.reg as usize]
        })
        .collect::<Vec<_>>();
    intervals.sort_by_key(|interval| (interval.start, interval.end, interval.reg));

    let mut active = Vec::<(usize, u8)>::new();

    for interval in intervals {
        active.retain(|(end, _)| *end >= interval.start);

        let mut occupied = [false; REG_COUNT];
        for &(_, reg) in &active {
            occupied[map[reg as usize] as usize] = true;
        }

        let physical = available_regs
            .iter()
            .copied()
            .find(|&candidate| !occupied[candidate as usize])
            .unwrap_or(interval.reg);

        map[interval.reg as usize] = physical;
        active.push((interval.end, interval.reg));
    }

    if map
        .iter()
        .enumerate()
        .all(|(index, &reg)| reg == index as u8)
    {
        return false;
    }

    let mut rewritten = insts.to_vec();
    for inst in &mut rewritten {
        if !rewrite_instruction_registers(inst, &map) {
            return false;
        }
    }

    insts.clone_from_slice(&rewritten);
    true
}
