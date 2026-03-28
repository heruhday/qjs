use std::collections::{BTreeSet, HashMap};

use crate::js_value::{JSValue, is_null, is_undefined, make_false, make_number, make_true, to_f64};
use crate::vm::Opcode;

const ACC: u8 = 255;
const REG_COUNT: usize = 256;

mod coalesce_registers;
mod constant_fold;
mod copy_propagation;
mod eliminate_dead_code;
mod fold_temporary_checks;
mod optimize_basic_peephole;
mod optimize_bytecode;
mod optimize_superinstructions;
mod reuse_registers_linear_scan;
mod run_fixed_point_round;
mod run_until_stable;
mod simplify_branches;

pub use self::coalesce_registers::coalesce_registers;
pub use self::copy_propagation::copy_propagation;
pub use self::eliminate_dead_code::eliminate_dead_code;
pub use self::fold_temporary_checks::fold_temporary_checks;
pub use self::optimize_bytecode::optimize_bytecode;
pub use self::reuse_registers_linear_scan::reuse_registers_linear_scan;
pub use self::simplify_branches::simplify_branches;

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
        let target = decode_branch_target(opcode, pc, a, b, c, sbx);

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
            | Opcode::JmpLte
            | Opcode::JmpLteFalse => {
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
                let sbx = self.sbx;
                (((sbx as u16) as u32) << 16) | ((self.a as u32) << 8) | self.opcode.as_u8() as u32
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

fn decode_branch_target(
    opcode: Opcode,
    pc: usize,
    a: u8,
    _b: u8,
    c: u8,
    sbx: i16,
) -> Option<usize> {
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
        Opcode::JmpEq | Opcode::JmpNeq | Opcode::JmpLt | Opcode::JmpLte | Opcode::JmpLteFalse => {
            next + c as i8 as isize
        }
        Opcode::EqJmpTrue | Opcode::LtJmp | Opcode::EqJmpFalse | Opcode::LteJmpLoop => {
            next + a as i8 as isize
        }
        _ => return None,
    };

    Some(target.max(0) as usize)
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
        Opcode::Jmp | Opcode::Ret | Opcode::RetU | Opcode::RetReg | Opcode::Throw
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

fn rewrite_load_move(insts: &mut [Instruction], first: usize, second: usize) -> bool {
    if insts[first].opcode != Opcode::LoadI || insts[second].opcode != Opcode::Mov {
        return false;
    }
    if insts[second].b != insts[first].a {
        return false;
    }

    let value = insts[first].sbx;
    insts[second] = Instruction::new_load_i(insts[second].a, value);
    true
}

fn fold_const_add(insts: &mut [Instruction], first: usize, second: usize, third: usize) -> bool {
    if insts[first].opcode != Opcode::LoadI
        || insts[second].opcode != Opcode::LoadI
        || insts[third].opcode != Opcode::Add
    {
        return false;
    }

    if insts[first].a == insts[second].a {
        return false;
    }

    if insts[third].b != insts[first].a || insts[third].c != insts[second].a {
        return false;
    }

    let folded = insts[first].sbx as i32 + insts[second].sbx as i32;
    if !(i16::MIN as i32..=i16::MAX as i32).contains(&folded) {
        return false;
    }

    insts[third] = Instruction::new_load_i(ACC, folded as i16);
    true
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
            | Opcode::AddStr => {
                live[ACC as usize] = false;
                live[insts[index].b as usize] = true;
                live[insts[index].c as usize] = true;
            }
            Opcode::AddAcc
            | Opcode::SubAcc
            | Opcode::MulAcc
            | Opcode::DivAcc
            | Opcode::AddStrAcc => {
                live[ACC as usize] = true;
                live[insts[index].b as usize] = true;
            }
            Opcode::AddAccImm8
            | Opcode::SubAccImm8
            | Opcode::MulAccImm8
            | Opcode::DivAccImm8
            | Opcode::IncAcc => {
                live[ACC as usize] = true;
            }
            Opcode::LoadThis
            | Opcode::Load0
            | Opcode::Load1
            | Opcode::LoadNull
            | Opcode::LoadTrue
            | Opcode::LoadFalse => {
                live[ACC as usize] = false;
                if insts[index].opcode == Opcode::LoadThis {
                    live[0] = true;
                }
            }
            Opcode::LoadAcc => {
                live[ACC as usize] = false;
                live[insts[index].a as usize] = true;
            }
            Opcode::JmpTrue | Opcode::JmpFalse => {
                live[insts[index].a as usize] = true;
            }
            Opcode::JmpEq
            | Opcode::JmpNeq
            | Opcode::JmpLt
            | Opcode::JmpLte
            | Opcode::JmpLteFalse => {
                live[insts[index].a as usize] = true;
                live[insts[index].b as usize] = true;
            }
            Opcode::LoopIncJmp => {
                live[insts[index].a as usize] = true;
                live[ACC as usize] = true;
            }
            Opcode::EqJmpTrue | Opcode::LtJmp | Opcode::EqJmpFalse | Opcode::LteJmpLoop => {
                live[insts[index].b as usize] = true;
                live[insts[index].c as usize] = true;
            }
            Opcode::Ret => {
                live[ACC as usize] = true;
            }
            Opcode::RetReg => {
                live[insts[index].a as usize] = true;
            }
            Opcode::RetU | Opcode::Jmp => {}
            _ => {
                live.fill(true);
            }
        }
    }

    changed
}

fn constant_fold_block(insts: &mut [Instruction], start: usize, end: usize) -> bool {
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

            if let (Some(&next_index), Some(&third_index)) =
                (live_indices.get(pos + 1), live_indices.get(pos + 2))
                && !insts[next_index].removed
                && !insts[third_index].removed
                && fold_const_add(insts, index, next_index, third_index)
            {
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
        }

        if !local_change {
            break;
        }

        changed = true;
    }

    changed
}

fn optimize_peephole_block(insts: &mut [Instruction], start: usize, end: usize) -> bool {
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

            if let Some(&next_index) = live_indices.get(pos + 1) {
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

                if insts[next_index].opcode == Opcode::Mov
                    && insts[next_index].a == insts[next_index].b
                {
                    insts[next_index].removed = true;
                    local_change = true;
                    continue;
                }

                if rewrite_load_move(insts, index, next_index) {
                    local_change = true;
                }

                // Pattern: LoadI + AddI -> LoadAdd
                if insts[index].opcode == Opcode::LoadI
                    && insts[next_index].opcode == Opcode::AddI
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadAdd,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadI + SubI -> LoadSub
                if insts[index].opcode == Opcode::LoadI
                    && insts[next_index].opcode == Opcode::SubI
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadSub,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadI + MulI -> LoadMul
                if insts[index].opcode == Opcode::LoadI
                    && insts[next_index].opcode == Opcode::MulI
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadMul,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadI + Inc -> LoadInc
                if insts[index].opcode == Opcode::LoadI
                    && insts[next_index].opcode == Opcode::Inc
                    && insts[next_index].a == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadInc,
                        a: insts[index].a,
                        b: 0,
                        c: 0,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadI + Dec -> LoadDec
                if insts[index].opcode == Opcode::LoadI
                    && insts[next_index].opcode == Opcode::Dec
                    && insts[next_index].a == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadDec,
                        a: insts[index].a,
                        b: 0,
                        c: 0,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadI + Eq -> LoadCmpEq
                if insts[index].opcode == Opcode::LoadI
                    && insts[next_index].opcode == Opcode::Eq
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadCmpEq,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadI + Lt -> LoadCmpLt
                if insts[index].opcode == Opcode::LoadI
                    && insts[next_index].opcode == Opcode::Lt
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadCmpLt,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadI + JmpFalse -> LoadJfalse
                if insts[index].opcode == Opcode::LoadI
                    && insts[next_index].opcode == Opcode::JmpFalse
                    && insts[next_index].a == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadJfalse,
                        a: insts[index].a,
                        b: 0,
                        c: 0,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: insts[next_index].target,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadK + GetProp + Eq -> LoadGetPropCmpEq
                if let Some(&third_index) = live_indices.get(pos + 2)
                    && insts[index].opcode == Opcode::LoadK
                    && insts[next_index].opcode == Opcode::GetProp
                    && insts[third_index].opcode == Opcode::Eq
                    && insts[next_index].b == insts[index].a
                    && insts[third_index].b == insts[next_index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadGetPropCmpEq,
                        a: insts[third_index].a,
                        b: insts[index].a,
                        c: insts[third_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    insts[third_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadK + GetProp -> LoadGetProp
                if insts[index].opcode == Opcode::LoadK
                    && insts[next_index].opcode == Opcode::GetProp
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadGetProp,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: GetProp + GetProp + GetProp -> GetProp3Ic
                if let Some(&third_index) = live_indices.get(pos + 2)
                    && insts[index].opcode == Opcode::GetProp
                    && insts[next_index].opcode == Opcode::GetProp
                    && insts[third_index].opcode == Opcode::GetProp
                    && insts[next_index].b == insts[index].a
                    && insts[third_index].b == insts[next_index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::GetProp3Ic,
                        a: insts[third_index].a,
                        b: insts[index].b,
                        c: insts[index].c,
                        bx: 0,
                        sbx: 0,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    insts[third_index].removed = true;
                    local_change = true;
                }

                // Pattern: GetProp + GetProp + Call -> CallMethod2Ic
                if let Some(&third_index) = live_indices.get(pos + 2)
                    && insts[index].opcode == Opcode::GetProp
                    && insts[next_index].opcode == Opcode::GetProp
                    && insts[third_index].opcode == Opcode::Call
                    && insts[next_index].b == insts[index].a
                    && insts[third_index].a == insts[next_index].a
                    && insts[third_index].b == 0
                {
                    insts[index] = Instruction {
                        opcode: Opcode::CallMethod2Ic,
                        a: insts[index].b,
                        b: insts[index].c,
                        c: insts[next_index].c,
                        bx: 0,
                        sbx: 0,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    insts[third_index].removed = true;
                    local_change = true;
                }

                // Pattern: GetProp + GetProp -> GetProp2Ic
                if insts[index].opcode == Opcode::GetProp
                    && insts[next_index].opcode == Opcode::GetProp
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::GetProp2Ic,
                        a: insts[next_index].a,
                        b: insts[index].b,
                        c: insts[index].c,
                        bx: 0,
                        sbx: 0,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadK + GetIdxFast -> GetElem
                if insts[index].opcode == Opcode::LoadK
                    && insts[next_index].opcode == Opcode::GetIdxFast
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::GetElem,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadK + SetIdxFast -> SetElem
                if insts[index].opcode == Opcode::LoadK
                    && insts[next_index].opcode == Opcode::SetIdxFast
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::SetElem,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: GetProp + GetIdxFast -> GetPropElem
                if insts[index].opcode == Opcode::GetProp
                    && insts[next_index].opcode == Opcode::GetIdxFast
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::GetPropElem,
                        a: insts[next_index].a,
                        b: insts[index].b,
                        c: insts[index].c,
                        bx: 0,
                        sbx: 0,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: GetProp + Call -> CallMethod*
                if insts[index].opcode == Opcode::GetProp
                    && insts[next_index].opcode == Opcode::Call
                    && insts[next_index].a == insts[index].a
                {
                    match insts[next_index].b {
                        0 => {
                            insts[index] = Instruction {
                                opcode: Opcode::CallMethodIc,
                                a: insts[index].b,
                                b: insts[index].c,
                                c: 0,
                                bx: 0,
                                sbx: 0,
                                target: None,
                                removed: false,
                            };
                            insts[next_index].removed = true;
                            local_change = true;
                        }
                        1 => {
                            insts[index] = Instruction {
                                opcode: Opcode::CallMethod1,
                                a: insts[index].b,
                                b: 0,
                                c: 0,
                                bx: insts[index].c as u16,
                                sbx: insts[index].c as i16,
                                target: None,
                                removed: false,
                            };
                            insts[next_index].removed = true;
                            local_change = true;
                        }
                        2 => {
                            insts[index] = Instruction {
                                opcode: Opcode::CallMethod2,
                                a: insts[index].b,
                                b: 0,
                                c: 0,
                                bx: insts[index].c as u16,
                                sbx: insts[index].c as i16,
                                target: None,
                                removed: false,
                            };
                            insts[next_index].removed = true;
                            local_change = true;
                        }
                        _ => {}
                    }
                }

                // Pattern: GetPropIc + Call -> GetPropIcCall
                if insts[index].opcode == Opcode::GetPropIc
                    && insts[next_index].opcode == Opcode::Call
                    && insts[next_index].a == insts[index].a
                    && insts[next_index].b == 0
                {
                    insts[index] = Instruction {
                        opcode: Opcode::GetPropIcCall,
                        a: insts[index].a,
                        b: insts[index].b,
                        c: insts[index].c,
                        bx: 0,
                        sbx: 0,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: IncAcc + JmpFalse -> IncJmpFalseLoop
                if insts[index].opcode == Opcode::IncAcc
                    && insts[next_index].opcode == Opcode::JmpFalse
                    && insts[next_index].a == ACC
                {
                    insts[index] = Instruction {
                        opcode: Opcode::IncJmpFalseLoop,
                        a: ACC,
                        b: 0,
                        c: 0,
                        bx: 0,
                        sbx: 0,
                        target: insts[next_index].target,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadK + Add -> LoadKAdd
                if insts[index].opcode == Opcode::LoadK
                    && insts[next_index].opcode == Opcode::Add
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadKAdd,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadK + Cmp -> LoadKCmp
                if insts[index].opcode == Opcode::LoadK
                    && insts[next_index].opcode == Opcode::Eq
                    && insts[next_index].b == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadKCmp,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: Cmp + Jmp -> CmpJmp
                if insts[index].opcode == Opcode::Eq
                    && insts[next_index].opcode == Opcode::JmpFalse
                    && insts[next_index].a == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::CmpJmp,
                        a: insts[index].a,
                        b: insts[index].b,
                        c: insts[index].c,
                        bx: 0,
                        sbx: 0,
                        target: insts[next_index].target,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadCmpEq + JmpFalse -> LoadCmpEqJfalse
                if insts[index].opcode == Opcode::LoadCmpEq
                    && insts[next_index].opcode == Opcode::JmpFalse
                    && insts[next_index].a == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadCmpEqJfalse,
                        a: insts[index].a,
                        b: insts[index].b,
                        c: insts[index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: insts[next_index].target,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadCmpLt + JmpFalse -> LoadCmpLtJfalse
                if insts[index].opcode == Opcode::LoadCmpLt
                    && insts[next_index].opcode == Opcode::JmpFalse
                    && insts[next_index].a == insts[index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadCmpLtJfalse,
                        a: insts[index].a,
                        b: insts[index].b,
                        c: insts[index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: insts[next_index].target,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadAcc + Add -> AddAccReg
                if insts[index].opcode == Opcode::LoadAcc
                    && insts[next_index].opcode == Opcode::Add
                    && insts[next_index].b == ACC
                {
                    insts[index] = Instruction {
                        opcode: Opcode::AddAccReg,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: 0,
                        sbx: 0,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }

                // Pattern: Call + Add -> Call1Add
                if insts[index].opcode == Opcode::Call
                    && insts[next_index].opcode == Opcode::Add
                    && insts[next_index].b == ACC
                {
                    insts[index] = Instruction {
                        opcode: Opcode::Call1Add,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: 0,
                        sbx: 0,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    local_change = true;
                }
            }

            if let (Some(&next_index), Some(&third_index)) =
                (live_indices.get(pos + 1), live_indices.get(pos + 2))
            {
                if insts[next_index].removed || insts[third_index].removed {
                    continue;
                }

                if fold_const_add(insts, index, next_index, third_index) {
                    local_change = true;
                }

                // Pattern: LoadI + Eq + JmpFalse -> LoadCmpEqJfalse
                if insts[index].opcode == Opcode::LoadI
                    && insts[next_index].opcode == Opcode::Eq
                    && insts[third_index].opcode == Opcode::JmpFalse
                    && insts[next_index].b == insts[index].a
                    && insts[third_index].a == insts[next_index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadCmpEqJfalse,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: insts[third_index].target,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    insts[third_index].removed = true;
                    local_change = true;
                }

                // Pattern: LoadI + Lt + JmpFalse -> LoadCmpLtJfalse
                if insts[index].opcode == Opcode::LoadI
                    && insts[next_index].opcode == Opcode::Lt
                    && insts[third_index].opcode == Opcode::JmpFalse
                    && insts[next_index].b == insts[index].a
                    && insts[third_index].a == insts[next_index].a
                {
                    insts[index] = Instruction {
                        opcode: Opcode::LoadCmpLtJfalse,
                        a: insts[next_index].a,
                        b: insts[index].a,
                        c: insts[next_index].c,
                        bx: insts[index].bx,
                        sbx: insts[index].sbx,
                        target: insts[third_index].target,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    insts[third_index].removed = true;
                    local_change = true;
                }

                // Pattern: Call + Call + Add -> Call2Add
                if let Some(&third_index) = live_indices.get(pos + 2)
                    && insts[index].opcode == Opcode::Call
                    && insts[next_index].opcode == Opcode::Call
                    && insts[third_index].opcode == Opcode::Add
                    && insts[next_index].a == insts[index].a
                    && insts[third_index].b == ACC
                {
                    insts[index] = Instruction {
                        opcode: Opcode::Call2Add,
                        a: insts[third_index].a,
                        b: insts[index].a,
                        c: insts[third_index].c,
                        bx: 0,
                        sbx: 0,
                        target: None,
                        removed: false,
                    };
                    insts[next_index].removed = true;
                    insts[third_index].removed = true;
                    local_change = true;
                }
            }
        }

        if !local_change {
            break;
        }

        changed = true;
    }

    changed
}

fn decode_program(bytecode: &[u32]) -> Vec<Instruction> {
    bytecode
        .iter()
        .enumerate()
        .map(|(pc, &raw)| Instruction::decode(pc, raw))
        .collect()
}

fn encode_program(insts: &[Instruction], mut constants: Vec<JSValue>) -> (Vec<u32>, Vec<JSValue>) {
    let boundary = build_boundary_map(insts);
    let bytecode: Vec<_> = insts
        .iter()
        .enumerate()
        .filter(|(_, inst)| !inst.removed)
        .map(|(pc, inst)| inst.encode(pc, &boundary))
        .collect();
    rewrite_switch_tables(insts, &mut constants, &boundary);
    (bytecode, constants)
}

fn has_removed_instructions(insts: &[Instruction]) -> bool {
    insts.iter().any(|inst| inst.removed)
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

fn push_unique_reg(regs: &mut Vec<u8>, reg: u8) {
    if !regs.contains(&reg) {
        regs.push(reg);
    }
}

fn push_call_bundle(regs: &mut Vec<u8>, base: u8, arg_count: u8) -> bool {
    let last = base as usize + arg_count as usize;
    if last >= ACC as usize {
        return false;
    }

    for reg in base..=base + arg_count {
        push_unique_reg(regs, reg);
    }

    true
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

fn build_instruction_semantics(
    insts: &[Instruction],
    constants: &[JSValue],
) -> Option<Vec<InstructionSemantics>> {
    let len = insts.len();
    let mut semantics = Vec::with_capacity(len);

    for (pc, inst) in insts.iter().enumerate() {
        let mut uses = Vec::new();
        let mut defs = Vec::new();
        let mut pinned = Vec::new();
        let mut successors = normal_successors(pc, len);

        match inst.opcode {
            Opcode::Mov => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, inst.b);
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
            | Opcode::CreateEnv => {
                push_unique_reg(&mut defs, inst.a);
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
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::ForIn => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::IteratorNext => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.a);
            }
            Opcode::Spread => {
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::SetGlobalIc
            | Opcode::SetGlobal
            | Opcode::SetUpval
            | Opcode::SetScope
            | Opcode::StoreName
            | Opcode::InitName => {
                push_unique_reg(&mut uses, inst.a);
            }
            Opcode::LoadName => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut defs, ACC);
            }
            Opcode::LoadArg => {
                push_unique_reg(&mut defs, inst.a);
            }
            Opcode::LoadAcc => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.a);
            }
            Opcode::LoadThis => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
            }
            Opcode::Load0
            | Opcode::Load1
            | Opcode::LoadNull
            | Opcode::LoadTrue
            | Opcode::LoadFalse
            | Opcode::IcMiss
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
            | Opcode::AssertNotDeepStrictEqual => {
                push_unique_reg(&mut defs, ACC);
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
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::AddAcc
            | Opcode::SubAcc
            | Opcode::MulAcc
            | Opcode::DivAcc
            | Opcode::AddStrAcc => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, ACC);
                push_unique_reg(&mut uses, inst.b);
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
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, ACC);
            }
            Opcode::AddI | Opcode::SubI | Opcode::MulI | Opcode::DivI | Opcode::ModI => {
                push_unique_reg(&mut defs, ACC);
                if inst.a != ACC {
                    push_unique_reg(&mut defs, inst.a);
                }
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::Mod => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::AddI32
            | Opcode::AddF64
            | Opcode::SubI32
            | Opcode::SubF64
            | Opcode::MulI32
            | Opcode::MulF64
            | Opcode::AddI32Fast
            | Opcode::AddF64Fast
            | Opcode::SubI32Fast
            | Opcode::MulI32Fast => {
                push_unique_reg(&mut defs, ACC);
                if inst.a != ACC {
                    push_unique_reg(&mut defs, inst.a);
                }
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::Neg | Opcode::Inc | Opcode::Dec | Opcode::ToPrimitive | Opcode::BitNot => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::GetPropAcc => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::SetPropAcc => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, ACC);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::GetProp | Opcode::GetSuper | Opcode::GetPropIc | Opcode::GetPropMono => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::SetProp | Opcode::SetSuper | Opcode::SetPropIc => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::GetIdxFast | Opcode::GetElem => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::GetIdxIc => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
                push_unique_reg(&mut pinned, inst.c);
            }
            Opcode::SetIdxFast | Opcode::SetElem => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::SetIdxIc => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
                push_unique_reg(&mut pinned, inst.c);
            }
            Opcode::GetLengthIc => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::ArrayPushAcc => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, ACC);
            }
            Opcode::Jmp => {
                successors = target_successors(inst.target, len);
            }
            Opcode::JmpTrue | Opcode::JmpFalse | Opcode::TestJmpTrue => {
                push_unique_reg(&mut uses, inst.a);
                successors = conditional_successors(pc, inst.target, len);
            }
            Opcode::JmpEq
            | Opcode::JmpNeq
            | Opcode::JmpLt
            | Opcode::JmpLte
            | Opcode::JmpLteFalse
            | Opcode::JmpI32Fast => {
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
                successors = conditional_successors(pc, inst.target, len);
            }
            Opcode::EqJmpTrue | Opcode::LtJmp | Opcode::EqJmpFalse | Opcode::LteJmpLoop => {
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
                successors = conditional_successors(pc, inst.target, len);
            }
            Opcode::LoopIncJmp => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, ACC);
                successors = conditional_successors(pc, inst.target, len);
            }
            Opcode::IncJmpFalseLoop => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, ACC);
                push_unique_reg(&mut uses, inst.a);
                successors = conditional_successors(pc, inst.target, len);
            }
            Opcode::Switch => {
                push_unique_reg(&mut uses, inst.a);
                successors = switch_targets(pc, inst.b as usize, constants);
                if successors.is_empty() {
                    successors = normal_successors(pc, len);
                }
            }
            Opcode::Ret => {
                push_unique_reg(&mut uses, ACC);
                successors.clear();
            }
            Opcode::RetReg => {
                push_unique_reg(&mut uses, inst.a);
                successors.clear();
            }
            Opcode::RetU => {
                successors.clear();
            }
            Opcode::Yield | Opcode::Await => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.a);
            }
            Opcode::ProfileType | Opcode::ProfileCall | Opcode::CheckType | Opcode::CheckStruct => {
                if inst.b != 0 || inst.c != 0 {
                    push_unique_reg(&mut uses, inst.b);
                } else {
                    push_unique_reg(&mut uses, ACC);
                }
            }
            Opcode::ProfileRet => {
                push_unique_reg(&mut uses, ACC);
            }
            Opcode::CheckIc => {
                push_unique_reg(&mut defs, ACC);
                if inst.b != 0 || inst.c != 0 {
                    push_unique_reg(&mut uses, inst.b);
                } else {
                    push_unique_reg(&mut uses, ACC);
                }
            }
            Opcode::IcInit | Opcode::IcUpdate => {
                if inst.b != 0 || inst.c != 0 {
                    push_unique_reg(&mut uses, inst.b);
                } else {
                    push_unique_reg(&mut uses, ACC);
                }
            }
            Opcode::LoopHint
            | Opcode::OsrEntry
            | Opcode::ProfileHotLoop
            | Opcode::OsrExit
            | Opcode::JitHint
            | Opcode::Enter
            | Opcode::Leave => {}
            Opcode::SafetyCheck => {
                if inst.a != 0 {
                    push_unique_reg(&mut uses, inst.a);
                } else {
                    push_unique_reg(&mut uses, ACC);
                }
            }
            Opcode::Call1SubI => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::Call
            | Opcode::TailCall
            | Opcode::Construct
            | Opcode::CallIc
            | Opcode::CallIcSuper
            | Opcode::CallMono => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                if !push_call_bundle(&mut uses, inst.a, inst.b)
                    || !push_call_bundle(&mut pinned, inst.a, inst.b)
                {
                    return None;
                }
            }
            Opcode::ProfileHotCall => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                if !push_call_bundle(&mut uses, inst.b, inst.c)
                    || !push_call_bundle(&mut pinned, inst.b, inst.c)
                {
                    return None;
                }
            }
            Opcode::CallRet => {
                push_unique_reg(&mut uses, 0);
                if !push_call_bundle(&mut uses, inst.a, inst.b)
                    || !push_call_bundle(&mut pinned, inst.a, inst.b)
                {
                    return None;
                }
                successors.clear();
            }
            Opcode::CallVar | Opcode::CallIcVar => {
                if inst.a as usize + 1 >= ACC as usize {
                    return None;
                }
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.a + 1);
                push_unique_reg(&mut pinned, inst.a);
                push_unique_reg(&mut pinned, inst.a + 1);
            }
            Opcode::Call0 => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                push_unique_reg(&mut uses, inst.a);
            }
            Opcode::Call1 => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::Call2 => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::CallMethod1 => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                if !push_call_bundle(&mut uses, inst.a, 1)
                    || !push_call_bundle(&mut pinned, inst.a, 1)
                {
                    return None;
                }
            }
            Opcode::CallMethod2 => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                if !push_call_bundle(&mut uses, inst.a, 2)
                    || !push_call_bundle(&mut pinned, inst.a, 2)
                {
                    return None;
                }
            }
            Opcode::GetPropIcCall | Opcode::GetPropCall => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::AddMov
            | Opcode::LoadAdd
            | Opcode::LoadSub
            | Opcode::LoadMul
            | Opcode::LoadCmpEq
            | Opcode::LoadCmpLt => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
                if matches!(inst.opcode, Opcode::AddMov) {
                    push_unique_reg(&mut defs, ACC);
                }
            }
            Opcode::GetPropAccCall => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::GetPropIcMov | Opcode::NewObjInitProp => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::GetPropAddImmSetPropIc => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::AddAccImm8Mov => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, ACC);
            }
            Opcode::LoadThisCall => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
            }
            Opcode::GetLengthIcCall => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::AddStrAccMov | Opcode::MulAccMov => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, ACC);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::LoadArgCall => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
            }
            Opcode::RetIfLteI => {
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::AddAccReg => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::Call1Add => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                push_unique_reg(&mut uses, ACC);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::Call2Add => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, 0);
                push_unique_reg(&mut uses, ACC);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.b);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::LoadKAdd => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, ACC);
            }
            Opcode::LoadKCmp | Opcode::LoadGetProp => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.a);
            }
            Opcode::LoadInc | Opcode::LoadDec => {
                push_unique_reg(&mut defs, inst.a);
                push_unique_reg(&mut uses, inst.b);
            }
            Opcode::CallMethodIc | Opcode::CallMethod2Ic => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.a);
            }
            Opcode::LoadGetPropCmpEq => {
                push_unique_reg(&mut defs, ACC);
                push_unique_reg(&mut uses, inst.a);
                push_unique_reg(&mut uses, inst.c);
            }
            Opcode::CmpJmp
            | Opcode::LoadJfalse
            | Opcode::LoadCmpEqJfalse
            | Opcode::LoadCmpLtJfalse
            | Opcode::GetProp2Ic
            | Opcode::GetProp3Ic
            | Opcode::GetPropElem
            | Opcode::Call3
            | Opcode::GetPropChainAcc
            | Opcode::Destructure
            | Opcode::Throw
            | Opcode::Try
            | Opcode::EndTry
            | Opcode::Catch
            | Opcode::Finally
            | Opcode::Reserved(_) => return None,
            _ => todo!("Opcode semantics not defined: {:?}", inst.opcode),
        }

        semantics.push(InstructionSemantics {
            uses,
            defs,
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
    pinned[ACC as usize] = true;

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

fn rewrite_instruction_registers(inst: &mut Instruction, map: &[u8; REG_COUNT]) {
    match inst.opcode {
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
        | Opcode::IteratorNext
        | Opcode::SetGlobalIc
        | Opcode::SetGlobal
        | Opcode::SetUpval
        | Opcode::SetScope
        | Opcode::StoreName
        | Opcode::InitName
        | Opcode::LoadAcc
        | Opcode::JmpTrue
        | Opcode::JmpFalse
        | Opcode::TestJmpTrue
        | Opcode::RetReg
        | Opcode::Yield
        | Opcode::Await
        | Opcode::ArrayPushAcc
        | Opcode::LoadKCmp
        | Opcode::LoadGetProp
        | Opcode::Call0
        | Opcode::CallMethod1
        | Opcode::CallMethod2
        | Opcode::CallMethodIc
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
        | Opcode::DeleteProp
        | Opcode::HasProp
        | Opcode::Keys
        | Opcode::GetProp
        | Opcode::GetSuper
        | Opcode::GetPropIc
        | Opcode::GetPropMono
        | Opcode::GetPropIcMov
        | Opcode::NewObjInitProp => {
            rewrite_register_with_map(&mut inst.a, map);
            rewrite_register_with_map(&mut inst.b, map);
        }
        Opcode::ForIn => {
            rewrite_register_with_map(&mut inst.a, map);
            rewrite_register_with_map(&mut inst.b, map);
        }
        Opcode::Spread
        | Opcode::SetProp
        | Opcode::SetSuper
        | Opcode::SetPropIc
        | Opcode::GetIdxFast
        | Opcode::GetIdxIc
        | Opcode::GetElem
        | Opcode::SetIdxFast
        | Opcode::SetIdxIc
        | Opcode::SetElem
        | Opcode::Add
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
        | Opcode::JmpLte
        | Opcode::JmpLteFalse
        | Opcode::JmpI32Fast
        | Opcode::EqJmpTrue
        | Opcode::LtJmp
        | Opcode::EqJmpFalse
        | Opcode::LteJmpLoop
        | Opcode::Call1
        | Opcode::LoadInc
        | Opcode::LoadDec
        | Opcode::CallMethod2Ic => {
            rewrite_register_with_map(&mut inst.a, map);
            rewrite_register_with_map(&mut inst.b, map);
            if !matches!(
                inst.opcode,
                Opcode::LoadInc | Opcode::LoadDec | Opcode::CallMethodIc | Opcode::CallMethod2Ic
            ) {
                rewrite_register_with_map(&mut inst.c, map);
            }
        }
        Opcode::Call2
        | Opcode::GetPropAcc
        | Opcode::SetPropAcc
        | Opcode::AddMov
        | Opcode::LoadAdd
        | Opcode::LoadSub
        | Opcode::LoadMul
        | Opcode::LoadCmpEq
        | Opcode::LoadCmpLt
        | Opcode::LoadGetPropCmpEq
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
        | Opcode::GetLengthIc
        | Opcode::GetLengthIcCall
        | Opcode::GetPropAccCall
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
        Opcode::LoopIncJmp
        | Opcode::Call1SubI
        | Opcode::GetPropIcCall
        | Opcode::GetPropCall
        | Opcode::Call1Add => {
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
        | Opcode::CallIcVar => {
            rewrite_register_with_map(&mut inst.a, map);
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
        | Opcode::GetPropAddImmSetPropIc
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
        Opcode::LoadName => {
            rewrite_register_with_map(&mut inst.a, map);
        }
        Opcode::Reserved(_)
        | Opcode::CmpJmp
        | Opcode::LoadJfalse
        | Opcode::LoadCmpEqJfalse
        | Opcode::LoadCmpLtJfalse
        | Opcode::GetProp2Ic
        | Opcode::GetProp3Ic
        | Opcode::GetPropElem
        | Opcode::Call3
        | Opcode::GetPropChainAcc
        | Opcode::Destructure
        | Opcode::Throw
        | Opcode::Try
        | Opcode::EndTry
        | Opcode::Catch
        | Opcode::Finally => {}
        _ => todo!("Opcode register rewrite not defined: {:?}", inst.opcode),
    }
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
                    Opcode::Ret | Opcode::RetU | Opcode::RetReg | Opcode::Throw
                )
            });
        if pass(insts, start, end, terminal) {
            changed = true;
        }
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
                invalidate_value_key(&mut available, &mut values, ACC);
                if let Some(key) = values[inst.a as usize] {
                    record_value_key(&mut available, &mut values, ACC, key);
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
            | Opcode::BitNot => {
                invalidate_value_key(&mut available, &mut values, ACC);
            }
            Opcode::Typeof
            | Opcode::ToNum
            | Opcode::ToStr
            | Opcode::IsUndef
            | Opcode::IsNull
            | Opcode::LoadArg
            | Opcode::GetScope
            | Opcode::SetScope
            | Opcode::LoadGlobalIc
            | Opcode::GetGlobal
            | Opcode::GetUpval
            | Opcode::NewObj
            | Opcode::NewArr
            | Opcode::NewFunc
            | Opcode::NewClass
            | Opcode::NewThis
            | Opcode::LoadClosure
            | Opcode::ResolveScope
            | Opcode::InitName
            | Opcode::TypeofName
            | Opcode::CreateEnv
            | Opcode::Keys
            | Opcode::ForIn
            | Opcode::IteratorNext => {
                invalidate_value_key(&mut available, &mut values, inst.a);
            }
            Opcode::LoadName => {
                invalidate_value_key(&mut available, &mut values, inst.a);
                invalidate_value_key(&mut available, &mut values, ACC);
            }
            Opcode::GetProp
            | Opcode::SetProp
            | Opcode::GetSuper
            | Opcode::SetSuper
            | Opcode::DeleteProp
            | Opcode::HasProp
            | Opcode::GetPropIc
            | Opcode::SetPropIc
            | Opcode::GetIdxFast
            | Opcode::SetIdxFast
            | Opcode::GetIdxIc
            | Opcode::SetIdxIc
            | Opcode::GetLengthIc => {
                invalidate_value_key(&mut available, &mut values, inst.a);
                invalidate_value_key(&mut available, &mut values, ACC);
            }
            Opcode::Ret
            | Opcode::RetU
            | Opcode::RetReg
            | Opcode::Jmp
            | Opcode::JmpTrue
            | Opcode::JmpFalse
            | Opcode::JmpEq
            | Opcode::JmpNeq
            | Opcode::JmpLt
            | Opcode::JmpLte
            | Opcode::JmpLteFalse
            | Opcode::EqJmpTrue
            | Opcode::LtJmp
            | Opcode::EqJmpFalse
            | Opcode::LoopIncJmp
            | Opcode::Try
            | Opcode::IncJmpFalseLoop
            | Opcode::IncAccJmp
            | Opcode::TestJmpTrue
            | Opcode::LteJmpLoop
            | Opcode::Yield
            | Opcode::Await
            | Opcode::Throw
            | Opcode::Switch
            | Opcode::LoopHint => {}
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
                known[ACC as usize] = known[inst.a as usize];
            }
            Opcode::LoadThis
            | Opcode::Load0
            | Opcode::Load1
            | Opcode::LoadTrue
            | Opcode::LoadFalse => {
                known[ACC as usize] = KnownValueKind::NonNullish;
            }
            Opcode::LoadNull => {
                known[ACC as usize] = KnownValueKind::Null;
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
            Opcode::Typeof
            | Opcode::ToNum
            | Opcode::ToStr
            | Opcode::LoadArg
            | Opcode::GetScope
            | Opcode::SetScope
            | Opcode::LoadGlobalIc
            | Opcode::GetGlobal
            | Opcode::GetUpval
            | Opcode::NewObj
            | Opcode::NewArr
            | Opcode::NewFunc
            | Opcode::NewClass
            | Opcode::NewThis
            | Opcode::LoadClosure
            | Opcode::ResolveScope
            | Opcode::InitName
            | Opcode::TypeofName
            | Opcode::CreateEnv
            | Opcode::Keys
            | Opcode::ForIn
            | Opcode::IteratorNext => {
                known[inst.a as usize] = KnownValueKind::Unknown;
            }
            Opcode::LoadName => {
                known[inst.a as usize] = KnownValueKind::Unknown;
                known[ACC as usize] = KnownValueKind::Unknown;
            }
            Opcode::GetProp
            | Opcode::SetProp
            | Opcode::GetSuper
            | Opcode::SetSuper
            | Opcode::DeleteProp
            | Opcode::HasProp
            | Opcode::GetPropIc
            | Opcode::SetPropIc
            | Opcode::GetIdxFast
            | Opcode::SetIdxFast
            | Opcode::GetIdxIc
            | Opcode::SetIdxIc
            | Opcode::GetLengthIc => {
                known[inst.a as usize] = KnownValueKind::Unknown;
                known[ACC as usize] = KnownValueKind::Unknown;
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
            | Opcode::Neg
            | Opcode::Inc
            | Opcode::Dec
            | Opcode::ToPrimitive
            | Opcode::BitNot => {
                known[ACC as usize] = KnownValueKind::Unknown;
            }
            Opcode::Ret
            | Opcode::RetU
            | Opcode::RetReg
            | Opcode::Jmp
            | Opcode::JmpTrue
            | Opcode::JmpFalse
            | Opcode::JmpEq
            | Opcode::JmpNeq
            | Opcode::JmpLt
            | Opcode::JmpLte
            | Opcode::JmpLteFalse
            | Opcode::EqJmpTrue
            | Opcode::LtJmp
            | Opcode::EqJmpFalse
            | Opcode::LoopIncJmp
            | Opcode::Try
            | Opcode::IncJmpFalseLoop
            | Opcode::IncAccJmp
            | Opcode::TestJmpTrue
            | Opcode::LteJmpLoop
            | Opcode::Yield
            | Opcode::Await
            | Opcode::Throw
            | Opcode::Switch
            | Opcode::LoopHint => {}
            _ => {
                known.fill(KnownValueKind::Unknown);
            }
        }
    }

    changed
}

fn copy_propagation_block(insts: &mut [Instruction], start: usize, end: usize) -> bool {
    let mut changed = false;
    let mut aliases = [None; REG_COUNT];

    for inst in &mut insts[start..end] {
        if inst.removed {
            continue;
        }

        match inst.opcode {
            Opcode::Mov => {
                changed |= rewrite_reg(&aliases, &mut inst.b);
                invalidate_alias(&mut aliases, inst.a);
                if inst.a != inst.b {
                    aliases[inst.a as usize] = Some(inst.b);
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
            | Opcode::AddStr => {
                changed |= rewrite_reg(&aliases, &mut inst.b);
                changed |= rewrite_reg(&aliases, &mut inst.c);
                invalidate_alias(&mut aliases, ACC);
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
                changed |= rewrite_reg(&aliases, &mut inst.b);
                invalidate_alias(&mut aliases, ACC);
            }
            Opcode::Typeof
            | Opcode::ToNum
            | Opcode::ToStr
            | Opcode::IsUndef
            | Opcode::IsNull
            | Opcode::LoadArg
            | Opcode::GetScope
            | Opcode::SetScope => {
                changed |= rewrite_reg(&aliases, &mut inst.b);
                invalidate_alias(&mut aliases, inst.a);
            }
            Opcode::GetProp
            | Opcode::GetSuper
            | Opcode::DeleteProp
            | Opcode::HasProp
            | Opcode::GetPropIc
            | Opcode::GetLengthIc => {
                changed |= rewrite_reg(&aliases, &mut inst.b);
                invalidate_alias(&mut aliases, inst.a);
            }
            Opcode::SetProp | Opcode::SetSuper | Opcode::SetPropIc => {
                changed |= rewrite_reg(&aliases, &mut inst.a);
                changed |= rewrite_reg(&aliases, &mut inst.b);
                invalidate_alias(&mut aliases, ACC);
            }
            Opcode::GetIdxFast | Opcode::GetIdxIc => {
                changed |= rewrite_reg(&aliases, &mut inst.b);
                changed |= rewrite_reg(&aliases, &mut inst.c);
                invalidate_alias(&mut aliases, inst.a);
            }
            Opcode::SetIdxFast | Opcode::SetIdxIc => {
                changed |= rewrite_reg(&aliases, &mut inst.a);
                changed |= rewrite_reg(&aliases, &mut inst.b);
                changed |= rewrite_reg(&aliases, &mut inst.c);
                invalidate_alias(&mut aliases, ACC);
            }
            Opcode::LoadAcc => {
                changed |= rewrite_reg(&aliases, &mut inst.a);
                invalidate_alias(&mut aliases, ACC);
                aliases[ACC as usize] = Some(inst.a);
            }
            Opcode::JmpTrue
            | Opcode::JmpFalse
            | Opcode::Yield
            | Opcode::Await
            | Opcode::Throw
            | Opcode::RetReg => {
                changed |= rewrite_reg(&aliases, &mut inst.a);
            }
            Opcode::JmpEq
            | Opcode::JmpNeq
            | Opcode::JmpLt
            | Opcode::JmpLte
            | Opcode::JmpLteFalse => {
                changed |= rewrite_reg(&aliases, &mut inst.a);
                changed |= rewrite_reg(&aliases, &mut inst.b);
            }
            Opcode::EqJmpTrue | Opcode::LtJmp | Opcode::EqJmpFalse | Opcode::LteJmpLoop => {
                changed |= rewrite_reg(&aliases, &mut inst.b);
                changed |= rewrite_reg(&aliases, &mut inst.c);
            }
            Opcode::LoadI
            | Opcode::LoadK
            | Opcode::LoadGlobalIc
            | Opcode::GetGlobal
            | Opcode::GetUpval
            | Opcode::NewObj
            | Opcode::NewArr
            | Opcode::NewFunc
            | Opcode::NewClass
            | Opcode::NewThis
            | Opcode::LoadClosure
            | Opcode::ResolveScope
            | Opcode::InitName
            | Opcode::TypeofName
            | Opcode::CreateEnv
            | Opcode::Keys
            | Opcode::ForIn
            | Opcode::IteratorNext => {
                invalidate_alias(&mut aliases, inst.a);
            }
            Opcode::LoadName => {
                invalidate_alias(&mut aliases, inst.a);
                invalidate_alias(&mut aliases, ACC);
            }
            Opcode::Ret | Opcode::RetU | Opcode::Jmp | Opcode::Switch | Opcode::LoopHint => {}
            _ => {
                aliases.fill(None);
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

fn build_boundary_map(insts: &[Instruction]) -> Vec<usize> {
    let mut boundary = vec![0; insts.len() + 1];
    let mut next = insts.iter().filter(|inst| !inst.removed).count();
    boundary[insts.len()] = next;

    for index in (0..insts.len()).rev() {
        if !insts[index].removed {
            next -= 1;
            boundary[index] = next;
        } else {
            boundary[index] = next;
        }
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

pub fn optimize_peephole(bytecode: Vec<u32>, constants: Vec<JSValue>) -> (Vec<u32>, Vec<JSValue>) {
    let mut insts = decode_program(&bytecode);
    let changed = run_block_pass(&mut insts, &constants, |insts, start, end, _terminal| {
        optimize_peephole_block(insts, start, end)
    });
    if !changed {
        return (bytecode, constants);
    }
    encode_program(&insts, constants)
}

pub fn relocate_jumps(bytecode: Vec<u32>, constants: Vec<JSValue>) -> (Vec<u32>, Vec<JSValue>) {
    let insts = decode_program(&bytecode);
    if !has_removed_instructions(&insts) {
        return encode_program(&insts, constants);
    }
    encode_program(&insts, constants)
}

pub fn optimize_compiled(
    mut compiled: crate::codegen::CompiledBytecode,
) -> crate::codegen::CompiledBytecode {
    if compiled.function_constants.is_empty() {
        let (bytecode, constants) = optimize_bytecode(compiled.bytecode, compiled.constants);
        compiled.bytecode = bytecode;
        compiled.constants = constants;
        return compiled;
    }

    let mut function_entries = compiled
        .function_constants
        .iter()
        .filter_map(|&slot| {
            compiled
                .constants
                .get(slot as usize)
                .and_then(|value| to_f64(*value))
                .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
                .map(|entry| (slot, entry as usize))
        })
        .collect::<Vec<_>>();
    function_entries.sort_by_key(|&(_, entry_pc)| entry_pc);

    if function_entries.is_empty() {
        let (bytecode, constants) = optimize_bytecode(compiled.bytecode, compiled.constants);
        compiled.bytecode = bytecode;
        compiled.constants = constants;
        return compiled;
    }

    let original_bytecode = compiled.bytecode;
    let mut constants = compiled.constants;
    let mut optimized = Vec::new();
    let mut cursor = 0usize;

    for (index, &(slot, entry_pc)) in function_entries.iter().enumerate() {
        if entry_pc > cursor {
            let (segment, next_constants) =
                optimize_bytecode(original_bytecode[cursor..entry_pc].to_vec(), constants);
            optimized.extend(segment);
            constants = next_constants;
        }

        let next_entry = function_entries
            .get(index + 1)
            .map(|&(_, next_entry)| next_entry)
            .unwrap_or(original_bytecode.len());
        let new_entry = optimized.len();
        if let Some(constant) = constants.get_mut(slot as usize) {
            *constant = make_number(new_entry as f64);
        }

        let (segment, next_constants) =
            optimize_bytecode(original_bytecode[entry_pc..next_entry].to_vec(), constants);
        optimized.extend(segment);
        constants = next_constants;
        cursor = next_entry;
    }

    if cursor < original_bytecode.len() {
        let (segment, next_constants) =
            optimize_bytecode(original_bytecode[cursor..].to_vec(), constants);
        optimized.extend(segment);
        constants = next_constants;
    }

    compiled.bytecode = optimized;
    compiled.constants = constants;
    compiled
}
